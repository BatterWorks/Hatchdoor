use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use crate::api_types::ErrorResponse;
use crate::app_state::{AppState, internal_error, refresh_now};
use crate::git::WriteRecord;
use crate::vault::{
    VaultIndex, WriteError, WriteOutcome, create_note, delete_note, move_or_rename_note,
    update_note,
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
pub struct DeleteNoteRequest {
    pub expected_content_hash: String,
}

#[derive(Debug, Serialize)]
pub struct WriteCapabilitiesResponse {
    pub enabled: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct WriteOutcomeResponse {
    pub ok: bool,
    pub slug: Option<String>,
    pub relative_path: Option<String>,
    pub content_hash: Option<String>,
    pub rewritten_notes: usize,
    pub moved_assets: usize,
    pub trashed_path: Option<String>,
    pub git_sync_warning: Option<String>,
}

pub async fn write_capabilities_handler(State(state): State<AppState>) -> impl IntoResponse {
    let warnings = if state.web_auth_enabled {
        Vec::new()
    } else {
        vec![
            "Frontend writes are enabled without requiring Hatchdoor web authentication; this is unauthenticated and should not be exposed to untrusted networks.".to_string(),
        ]
    };
    (
        StatusCode::OK,
        Json(WriteCapabilitiesResponse {
            enabled: true,
            warnings,
        }),
    )
        .into_response()
}

pub async fn create_note_handler(
    State(state): State<AppState>,
    payload: Result<Json<CreateNoteRequest>, JsonRejection>,
) -> impl IntoResponse {
    let payload = match write_payload(payload) {
        Ok(payload) => payload,
        Err(err) => return err.into_response(),
    };
    let relative_path = match non_empty_input("relative_path", payload.relative_path) {
        Ok(relative_path) => relative_path,
        Err(err) => return err.into_response(),
    };

    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let outcome = match create_note(&state.vault_path, &relative_path, &payload.content, false) {
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
    let outcome = match update_note(&entry, &payload.content, &payload.expected_content_hash) {
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
    let outcome = match move_or_rename_note(
        &state.vault_path,
        &index,
        &entry,
        &target_relative_path,
        &payload.expected_content_hash,
    ) {
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
    let outcome = match move_or_rename_note(
        &state.vault_path,
        &index,
        &entry,
        &target_relative_path,
        &payload.expected_content_hash,
    ) {
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
    let outcome = match move_or_rename_note(
        &state.vault_path,
        &index,
        &entry,
        &target_relative_path,
        &payload.expected_content_hash,
    ) {
        Ok(outcome) => outcome,
        Err(err) => return write_error_response(err),
    };

    match finalize_note_write_response(&state, "move_rename", outcome).await {
        Ok(response) => response.into_response(),
        Err(err) => err.into_response(),
    }
}

pub async fn delete_note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    payload: Result<Json<DeleteNoteRequest>, JsonRejection>,
) -> impl IntoResponse {
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
    let outcome = match delete_note(
        &state.vault_path,
        &index,
        &entry,
        &payload.expected_content_hash,
    ) {
        Ok(outcome) => outcome,
        Err(err) => return write_error_response(err),
    };

    match finalize_note_write_response(&state, "delete", outcome).await {
        Ok(response) => response.into_response(),
        Err(err) => err.into_response(),
    }
}

async fn current_index(state: &AppState) -> Result<VaultIndex, (StatusCode, Json<ErrorResponse>)> {
    let vault_path = state.vault_path.clone();
    match tokio::task::spawn_blocking(move || VaultIndex::build(&vault_path)).await {
        Ok(Ok(index)) => Ok(index),
        Ok(Err(error)) => Err(internal_error(format!(
            "failed to index vault at '{}': {error}",
            state.vault_path.display()
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
    record_note_write(state, op, &outcome);
    refresh_after_write(state).await?;
    let refreshed_index = current_index(state).await?;
    let response =
        write_response_from_outcome(&refreshed_index, outcome, git_sync_warning(state).await)?;
    Ok((StatusCode::OK, Json(response)))
}

async fn refresh_after_write(state: &AppState) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    refresh_now(state)
        .await
        .map_err(|(_status, body)| internal_error(body.0.error))
}

async fn git_sync_warning(state: &AppState) -> Option<String> {
    let handle = state.git_sync.as_ref()?;
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

fn record_note_write(state: &AppState, op: &str, outcome: &WriteOutcome) {
    let target = outcome
        .relative_path
        .clone()
        .or_else(|| outcome.slug.clone())
        .unwrap_or_else(|| "note".to_string());
    state.record_vault_write(WriteRecord {
        op: op.to_string(),
        target,
        affected_paths: outcome.affected_paths.clone(),
        summary: None,
    });
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

    Ok(WriteOutcomeResponse {
        ok: true,
        slug,
        relative_path: outcome.relative_path,
        content_hash: outcome.content_hash,
        rewritten_notes: outcome.rewritten_notes,
        moved_assets: outcome.moved_assets,
        trashed_path: outcome.trashed_path,
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
        Err(rejection) => Err((
            StatusCode::BAD_REQUEST,
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
