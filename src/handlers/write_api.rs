use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::api_types::ErrorResponse;
use crate::app_state::{AppState, internal_error, refresh_now, vault_unavailable};
use crate::git::WriteRecord;
use crate::vault::{
    AttachmentInfo, AttachmentOutcome, VaultIndex, WriteError, WriteOutcome, archive_note,
    create_note, delete_note, import_attachment_bytes, move_or_rename_note, update_note,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateNoteRequest {
    pub relative_path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateNoteRequest {
    pub content: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameNoteRequest {
    pub new_title: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveNoteRequest {
    pub target_folder: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveRenameNoteRequest {
    pub target_relative_path: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveNoteRequest {
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteNoteRequest {
    pub expected_content_hash: String,
}

#[derive(Debug, Serialize)]
pub struct WriteCapabilitiesResponse {
    pub enabled: bool,
    pub settings_enabled: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct WriteOutcomeResponse {
    pub ok: bool,
    pub slug: Option<String>,
    pub relative_path: Option<String>,
    pub content_hash: Option<String>,
    pub quality_warnings: Vec<String>,
    pub rewritten_notes: usize,
    pub moved_assets: usize,
    pub trashed_path: Option<String>,
    /// The resulting note's layer (`None` = default surface).
    pub layer: Option<String>,
    pub git_sync_warning: Option<String>,
}

#[derive(Debug, Serialize)]
struct AttachmentOutcomeResponse {
    pub ok: bool,
    pub attachment: AttachmentInfo,
    pub rewritten_notes: usize,
    pub trashed_path: Option<String>,
    pub cleanup_warning: Option<String>,
    pub git_sync_warning: Option<String>,
}

pub async fn write_capabilities_handler(State(state): State<AppState>) -> impl IntoResponse {
    if state.demo_mode {
        return (
            StatusCode::OK,
            Json(WriteCapabilitiesResponse {
                enabled: false,
                settings_enabled: false,
                warnings: vec![
                    "Hatchdoor demo mode is read-only; browser write features are disabled."
                        .to_string(),
                ],
            }),
        )
            .into_response();
    }

    let lifecycle_allows_mutation = state.startup.snapshot().capabilities.mutate;
    let Some(vault_path) = state.vault_path().await else {
        return vault_unavailable().into_response();
    };
    let vault_writable = vault_path_is_writable(&vault_path);
    let mut warnings = Vec::new();
    if lifecycle_allows_mutation && vault_writable && !state.web_auth_enabled {
        warnings.push(
            "Frontend writes are enabled without requiring Hatchdoor web authentication; this is unauthenticated and should not be exposed to untrusted networks.".to_string(),
        );
    }
    if !vault_writable {
        warnings
            .push("Vault path is not writable; browser write features are disabled.".to_string());
    }
    if !lifecycle_allows_mutation {
        warnings.push(
            "The vault lifecycle is read-only; browser write features are disabled.".to_string(),
        );
    }
    (
        StatusCode::OK,
        Json(WriteCapabilitiesResponse {
            enabled: lifecycle_allows_mutation && vault_writable,
            settings_enabled: true,
            warnings,
        }),
    )
        .into_response()
}

fn vault_path_is_writable(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| !metadata.permissions().readonly())
        .unwrap_or(false)
}

pub(crate) fn reject_demo_mode_write(state: &AppState) -> Option<Response> {
    if state.demo_mode {
        return Some(
            (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: "Hatchdoor demo mode is read-only; write operations are disabled."
                        .to_string(),
                }),
            )
                .into_response(),
        );
    }
    (!state.startup.snapshot().capabilities.mutate).then(|| {
        (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "The vault lifecycle is read-only; write operations are disabled."
                    .to_string(),
            }),
        )
            .into_response()
    })
}

/// Refuse a write whose target path matches a noise-exclusion pattern: the index
/// applies the same matcher, so the file would land on disk yet be invisible to
/// every read surface. Mirrors the MCP write path's `refuse_noise_write`.
fn reject_noise_write(state: &AppState, path: &str) -> Option<Response> {
    let scan_config = match state.live_scan_config() {
        Ok(scan_config) => scan_config,
        Err(error) => return Some(crate::app_state::internal_error(error).into_response()),
    };
    scan_config
        .exclude
        .is_excluded(std::path::Path::new(path.trim()), false)
        .then(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "'{path}' matches a Hatchdoor noise-exclusion pattern and would be \
                         ignored by the index; choose a path outside the excluded set."
                    ),
                }),
            )
                .into_response()
        })
}

pub async fn upload_attachment_handler(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if let Some(response) = reject_demo_mode_write(&state) {
        return response;
    }

    let mut target_relative_path: Option<String> = None;
    let mut file_bytes: Option<Vec<u8>> = None;

    while let Some(field) = match multipart.next_field().await {
        Ok(field) => field,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("invalid multipart upload: {error}"),
                }),
            )
                .into_response();
        }
    } {
        let name = field.name().unwrap_or("").to_string();
        if name == "target_relative_path" {
            let value = match field.text().await {
                Ok(value) => value,
                Err(error) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("invalid target_relative_path field: {error}"),
                        }),
                    )
                        .into_response();
                }
            };
            target_relative_path = Some(value);
        } else if name == "file" {
            let bytes = match field.bytes().await {
                Ok(bytes) => bytes,
                Err(error) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("invalid file field: {error}"),
                        }),
                    )
                        .into_response();
                }
            };
            file_bytes = Some(bytes.to_vec());
        }
    }

    let target_relative_path = match non_empty_input(
        "target_relative_path",
        target_relative_path.unwrap_or_default(),
    ) {
        Ok(path) => path,
        Err(err) => return err.into_response(),
    };
    if let Some(response) = reject_noise_write(&state, &target_relative_path) {
        return response;
    }
    let file_bytes = match file_bytes {
        Some(bytes) if !bytes.is_empty() => bytes,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "file field is required".to_string(),
                }),
            )
                .into_response();
        }
    };

    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let Some(vault_path) = state.vault_path().await else {
        return vault_unavailable().into_response();
    };
    let snapshot = state.runtime_snapshot();
    let max_attachment_bytes = match AppState::runtime_mcp_config(&snapshot) {
        Ok(config) => config.max_attachment_bytes,
        Err(error) => return internal_error(error).into_response(),
    };
    let outcome = match run_write_op(move || {
        import_attachment_bytes(
            &vault_path,
            &target_relative_path,
            &file_bytes,
            max_attachment_bytes,
            false,
        )
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => return write_error_response(err),
    };

    match finalize_attachment_write_response(&state, outcome).await {
        Ok(response) => response.into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn create_note_handler(
    State(state): State<AppState>,
    payload: Result<Json<CreateNoteRequest>, JsonRejection>,
) -> impl IntoResponse {
    if let Some(response) = reject_demo_mode_write(&state) {
        return response;
    }

    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(err) => return err.into_response(),
    };
    let relative_path = match non_empty_input("relative_path", payload.relative_path) {
        Ok(relative_path) => relative_path,
        Err(err) => return err.into_response(),
    };
    if let Some(response) = reject_noise_write(&state, &relative_path) {
        return response;
    }

    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let Some(vault_path) = state.vault_path().await else {
        return vault_unavailable().into_response();
    };
    let outcome = match run_write_op(move || {
        create_note(&vault_path, &relative_path, &payload.content, false)
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => return write_error_response(err),
    };

    match finalize_note_write_response(&state, "create", outcome).await {
        Ok(response) => response.into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn update_note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    payload: Result<Json<UpdateNoteRequest>, JsonRejection>,
) -> impl IntoResponse {
    if let Some(response) = reject_demo_mode_write(&state) {
        return response;
    }

    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(err) => return err.into_response(),
    };
    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let index = match current_index(&state).await {
        Ok(index) => index,
        Err(err) => return err.into_response(),
    };
    let entry = match note_entry(&index, &slug) {
        Ok(entry) => entry,
        Err(err) => return err.into_response(),
    };
    let outcome = match run_write_op(move || {
        update_note(&entry, &payload.content, &payload.expected_content_hash)
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => return write_error_response(err),
    };

    match finalize_note_write_response(&state, "update", outcome).await {
        Ok(response) => response.into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn rename_note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    payload: Result<Json<RenameNoteRequest>, JsonRejection>,
) -> impl IntoResponse {
    if let Some(response) = reject_demo_mode_write(&state) {
        return response;
    }

    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(err) => return err.into_response(),
    };
    let new_title = match non_empty_input("new_title", payload.new_title) {
        Ok(new_title) => new_title,
        Err(err) => return err.into_response(),
    };
    if new_title.contains('/') || new_title.contains('\\') {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "new_title cannot contain path separators".to_string(),
            }),
        )
            .into_response();
    }

    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let index = match current_index(&state).await {
        Ok(index) => index,
        Err(err) => return err.into_response(),
    };
    let entry = match note_entry(&index, &slug) {
        Ok(entry) => entry,
        Err(err) => return err.into_response(),
    };
    let target_relative_path = replace_filename(&entry.relative_path, &new_title);
    if let Some(response) = reject_noise_write(&state, &target_relative_path) {
        return response;
    }
    let Some(vault_path) = state.vault_path().await else {
        return vault_unavailable().into_response();
    };
    let outcome = match run_write_op(move || {
        move_or_rename_note(
            &vault_path,
            &index,
            &entry,
            &target_relative_path,
            &payload.expected_content_hash,
        )
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => return write_error_response(err),
    };

    match finalize_note_write_response(&state, "rename", outcome).await {
        Ok(response) => response.into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn move_note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    payload: Result<Json<MoveNoteRequest>, JsonRejection>,
) -> impl IntoResponse {
    if let Some(response) = reject_demo_mode_write(&state) {
        return response;
    }

    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(err) => return err.into_response(),
    };
    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let index = match current_index(&state).await {
        Ok(index) => index,
        Err(err) => return err.into_response(),
    };
    let entry = match note_entry(&index, &slug) {
        Ok(entry) => entry,
        Err(err) => return err.into_response(),
    };
    let target_folder = payload.target_folder.trim().trim_matches('/');
    let file_name = entry
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or(&entry.relative_path);
    let target_relative_path = if target_folder.is_empty() {
        file_name.to_string()
    } else {
        format!("{target_folder}/{file_name}")
    };
    if let Some(response) = reject_noise_write(&state, &target_relative_path) {
        return response;
    }
    let Some(vault_path) = state.vault_path().await else {
        return vault_unavailable().into_response();
    };
    let expected_content_hash = payload.expected_content_hash;
    let outcome = match run_write_op(move || {
        move_or_rename_note(
            &vault_path,
            &index,
            &entry,
            &target_relative_path,
            &expected_content_hash,
        )
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => return write_error_response(err),
    };

    match finalize_note_write_response(&state, "move", outcome).await {
        Ok(response) => response.into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn move_rename_note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    payload: Result<Json<MoveRenameNoteRequest>, JsonRejection>,
) -> impl IntoResponse {
    if let Some(response) = reject_demo_mode_write(&state) {
        return response;
    }

    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(err) => return err.into_response(),
    };
    let target_relative_path =
        match non_empty_input("target_relative_path", payload.target_relative_path) {
            Ok(target_relative_path) => target_relative_path,
            Err(err) => return err.into_response(),
        };

    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let index = match current_index(&state).await {
        Ok(index) => index,
        Err(err) => return err.into_response(),
    };
    let entry = match note_entry(&index, &slug) {
        Ok(entry) => entry,
        Err(err) => return err.into_response(),
    };
    if let Some(response) = reject_noise_write(&state, &target_relative_path) {
        return response;
    }
    let Some(vault_path) = state.vault_path().await else {
        return vault_unavailable().into_response();
    };
    let outcome = match run_write_op(move || {
        move_or_rename_note(
            &vault_path,
            &index,
            &entry,
            &target_relative_path,
            &payload.expected_content_hash,
        )
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => return write_error_response(err),
    };

    match finalize_note_write_response(&state, "move_rename", outcome).await {
        Ok(response) => response.into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn archive_note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    payload: Result<Json<ArchiveNoteRequest>, JsonRejection>,
) -> impl IntoResponse {
    if let Some(response) = reject_demo_mode_write(&state) {
        return response;
    }

    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(err) => return err.into_response(),
    };

    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let index = match current_index(&state).await {
        Ok(index) => index,
        Err(err) => return err.into_response(),
    };
    let entry = match note_entry(&index, &slug) {
        Ok(entry) => entry,
        Err(err) => return err.into_response(),
    };
    let snapshot = state.runtime_snapshot();
    let archive_prefix = match AppState::runtime_archive_prefix(&snapshot) {
        Ok(prefix) => prefix,
        Err(error) => return internal_error(error).into_response(),
    };
    let archive_folder = archive_prefix.trim().trim_matches('/');
    let file_name = entry
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or(&entry.relative_path);
    let target_relative_path = format!("{archive_folder}/{file_name}");
    if let Some(response) = reject_noise_write(&state, &target_relative_path) {
        return response;
    }
    let Some(vault_path) = state.vault_path().await else {
        return vault_unavailable().into_response();
    };
    let outcome = match run_write_op(move || {
        archive_note(
            &vault_path,
            &index,
            &entry,
            &archive_prefix,
            &payload.expected_content_hash,
        )
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => return write_error_response(err),
    };

    match finalize_note_write_response(&state, "archive", outcome).await {
        Ok(response) => response.into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn delete_note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    payload: Result<Json<DeleteNoteRequest>, JsonRejection>,
) -> impl IntoResponse {
    if let Some(response) = reject_demo_mode_write(&state) {
        return response;
    }

    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(err) => return err.into_response(),
    };
    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let index = match current_index(&state).await {
        Ok(index) => index,
        Err(err) => return err.into_response(),
    };
    let entry = match note_entry(&index, &slug) {
        Ok(entry) => entry,
        Err(err) => return err.into_response(),
    };
    let Some(vault_path) = state.vault_path().await else {
        return vault_unavailable().into_response();
    };
    let outcome = match run_write_op(move || {
        delete_note(&vault_path, &index, &entry, &payload.expected_content_hash)
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => return write_error_response(err),
    };

    match finalize_note_write_response(&state, "delete", outcome).await {
        Ok(response) => response.into_response(),
        Err(err) => err.into_response(),
    }
}

/// Run a synchronous vault write op on the blocking pool. Moves/deletes rewrite
/// every backlinking note, which must not stall a tokio worker; a panic maps to
/// a `WriteError` instead of unwinding the handler.
async fn run_write_op<T>(
    op: impl FnOnce() -> Result<T, WriteError> + Send + 'static,
) -> Result<T, WriteError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .unwrap_or_else(|join_error| {
            Err(WriteError::Io(format!("write task panicked: {join_error}")))
        })
}

async fn current_index(state: &AppState) -> Result<VaultIndex, (StatusCode, Json<ErrorResponse>)> {
    let vault_path = state.vault_path().await.ok_or_else(vault_unavailable)?;
    let vault_display = vault_path.display().to_string();
    let scan_config = state.live_scan_config().map_err(internal_error)?;
    match tokio::task::spawn_blocking(move || {
        VaultIndex::build_with_config(&vault_path, &scan_config)
    })
    .await
    {
        Ok(Ok(index)) => Ok(index),
        Ok(Err(error)) => Err(internal_error(format!(
            "failed to index vault at '{}': {error}",
            vault_display
        ))),
        Err(join_error) => Err(internal_error(format!(
            "vault index build panicked: {join_error}"
        ))),
    }
}

fn note_entry(
    index: &VaultIndex,
    slug: &str,
) -> Result<crate::vault::NoteEntry, (StatusCode, Json<ErrorResponse>)> {
    let slug = slug.trim();
    if slug.is_empty() {
        return Err(note_not_found_response(slug));
    }
    index
        .find_by_slug(slug)
        .cloned()
        .ok_or_else(|| note_not_found_response(slug))
}

async fn finalize_note_write_response(
    state: &AppState,
    op: &str,
    outcome: WriteOutcome,
) -> Result<(StatusCode, Json<WriteOutcomeResponse>), (StatusCode, Json<ErrorResponse>)> {
    record_note_write(state, op, &outcome).await;
    refresh_after_write(state).await?;
    let refreshed_index = current_index(state).await?;
    let response =
        write_response_from_outcome(&refreshed_index, outcome, git_sync_warning(state).await)?;
    Ok((StatusCode::OK, Json(response)))
}

async fn finalize_attachment_write_response(
    state: &AppState,
    outcome: AttachmentOutcome,
) -> Result<(StatusCode, Json<AttachmentOutcomeResponse>), (StatusCode, Json<ErrorResponse>)> {
    state
        .record_vault_write(WriteRecord {
            op: "upload_attachment".to_string(),
            target: outcome.attachment.relative_path.clone(),
            affected_paths: outcome.affected_paths.clone(),
            summary: None,
        })
        .await;
    refresh_after_write(state).await?;
    Ok((
        StatusCode::OK,
        Json(AttachmentOutcomeResponse {
            ok: true,
            attachment: outcome.attachment,
            rewritten_notes: outcome.rewritten_notes,
            trashed_path: outcome.trashed_path,
            cleanup_warning: outcome.cleanup_warning,
            git_sync_warning: git_sync_warning(state).await,
        }),
    ))
}

async fn refresh_after_write(state: &AppState) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    refresh_now(state)
        .await
        .map_err(|(_status, body)| internal_error(body.0.error))
}

async fn git_sync_warning(state: &AppState) -> Option<String> {
    let sync = state.git_sync.read().await;
    let handle = sync.as_ref()?;
    let guard = handle.status();
    let snapshot = guard.read().await;
    if snapshot.last_ok {
        None
    } else {
        snapshot
            .last_error
            .clone()
            .map(|error| format!("git sync has not succeeded since: {error}"))
    }
}

async fn record_note_write(state: &AppState, op: &str, outcome: &WriteOutcome) {
    let target = outcome
        .relative_path
        .clone()
        .or_else(|| outcome.slug.clone())
        .unwrap_or_else(|| "note".to_string());
    state
        .record_vault_write(WriteRecord {
            op: op.to_string(),
            target,
            affected_paths: outcome.affected_paths.clone(),
            summary: None,
        })
        .await;
}

fn write_response_from_outcome(
    index: &VaultIndex,
    outcome: WriteOutcome,
    git_sync_warning: Option<String>,
) -> Result<WriteOutcomeResponse, (StatusCode, Json<ErrorResponse>)> {
    let slug = match (&outcome.slug, &outcome.relative_path) {
        (Some(slug), _) => Some(slug.clone()),
        (None, Some(relative_path)) => index
            .ordered_entries()
            .into_iter()
            .find(|entry| entry.relative_path == *relative_path)
            .map(|entry| entry.slug),
        (None, None) => None,
    };

    if outcome.relative_path.is_some() && slug.is_none() {
        return Err(internal_error(
            "note write completed but refreshed index did not contain the note",
        ));
    }

    let layer = slug
        .as_deref()
        .and_then(|slug| index.find_by_slug(slug))
        .and_then(|entry| entry.layer.clone());

    Ok(WriteOutcomeResponse {
        ok: true,
        slug,
        relative_path: outcome.relative_path,
        content_hash: outcome.content_hash,
        quality_warnings: outcome.quality_warnings,
        rewritten_notes: outcome.rewritten_notes,
        moved_assets: outcome.moved_assets,
        trashed_path: outcome.trashed_path,
        layer,
        git_sync_warning,
    })
}

fn write_error_response(error: WriteError) -> axum::response::Response {
    match error {
        WriteError::Conflict(message) => {
            (StatusCode::CONFLICT, Json(ErrorResponse { error: message })).into_response()
        }
        WriteError::InvalidInput(message) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
        WriteError::Io(message) => internal_error(message).into_response(),
    }
}

fn note_not_found_response(slug: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: format!("Note not found: {slug}"),
        }),
    )
}

fn write_payload<T>(
    payload: Result<Json<T>, JsonRejection>,
) -> Result<T, (StatusCode, Json<ErrorResponse>)> {
    match payload {
        Ok(Json(payload)) => Ok(payload),
        // Preserve the rejection's real status (e.g. 413 for a body over the
        // length limit, 415 for a bad content-type) instead of flattening every
        // JSON rejection to 400; the {error} body is unchanged.
        Err(rejection) => Err((
            rejection.status(),
            Json(ErrorResponse {
                error: rejection.body_text(),
            }),
        )),
    }
}

fn non_empty_input(
    field: &str,
    value: String,
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("{field} cannot be empty"),
            }),
        ));
    }
    Ok(trimmed.to_string())
}

fn replace_filename(relative_path: &str, new_title: &str) -> String {
    let directory = relative_path.rsplit_once('/').map(|(dir, _)| dir);
    match directory {
        Some(dir) if !dir.is_empty() => format!("{dir}/{new_title}.md"),
        _ => format!("{new_title}.md"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_write_op_passes_through_the_ops_result() {
        let ok: Result<u32, WriteError> = run_write_op(|| Ok(7)).await;
        assert_eq!(ok, Ok(7));

        let conflict: Result<u32, WriteError> =
            run_write_op(|| Err(WriteError::Conflict("taken".to_string()))).await;
        assert_eq!(conflict, Err(WriteError::Conflict("taken".to_string())));
    }

    #[tokio::test]
    async fn run_write_op_maps_a_panicking_op_to_an_io_write_error() {
        let result: Result<(), WriteError> = run_write_op(|| panic!("boom")).await;
        match result {
            Err(WriteError::Io(message)) => assert!(message.contains("panicked")),
            other => panic!("expected Io error, got {other:?}"),
        }
    }
}
