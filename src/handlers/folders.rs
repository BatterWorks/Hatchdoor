//! `GET` and `POST /api/v1/folders`: the HTTP adapter over the folder listing
//! (ADR-41) and the one folder it may create (ADR-44).
//!
//! Both touch the filesystem, so they run on the blocking pool. The route is
//! mounted behind the web token and refused in demo mode by `src/server.rs`;
//! this adapter only parses the request and shapes the answer.

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::folder_listing::{
    FolderCreateError, FolderListingError, ListingLimits, create_folder, list_folders,
};
use crate::handlers::vaults::{
    VaultApiError, internal_error_response, json_rejection_response, query_rejection_response,
};

#[derive(Debug, Deserialize)]
pub struct FolderQuery {
    /// Relative to the Vault mount root; absent or empty lists the root.
    #[serde(default)]
    pub path: String,
}

pub async fn list_folders_handler(
    State(state): State<AppState>,
    query: Result<Query<FolderQuery>, QueryRejection>,
) -> Response {
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => return query_rejection_response(error),
    };
    let root = state.vault_mount_root.clone();
    let registry = state.vault_registry.clone();
    let listing = tokio::task::spawn_blocking(move || {
        list_folders(&root, &query.path, &registry, ListingLimits::default())
    })
    .await;
    match listing {
        Ok(Ok(listing)) => Json(listing).into_response(),
        Ok(Err(error)) => VaultApiError::new(error.code(), error.message(), None, false)
            .respond(error_status(error)),
        Err(error) => internal_error_response(format!("folder listing task failed: {error}"), None),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateFolderRequest {
    /// The folder to make it in, relative to the Vault mount root; absent or
    /// empty is the root itself.
    #[serde(default)]
    pub parent: String,
    /// The new folder's name: one path segment.
    pub name: String,
}

pub async fn create_folder_handler(
    State(state): State<AppState>,
    request: Result<Json<CreateFolderRequest>, JsonRejection>,
) -> Response {
    let Json(request) = match request {
        Ok(request) => request,
        Err(error) => return json_rejection_response(error),
    };
    let root = state.vault_mount_root.clone();
    let registry = state.vault_registry.clone();
    let created = tokio::task::spawn_blocking(move || {
        create_folder(&root, &request.parent, &request.name, &registry)
    })
    .await;
    match created {
        Ok(Ok(folder)) => (StatusCode::CREATED, Json(folder)).into_response(),
        Ok(Err(error)) => VaultApiError::new(error.code(), error.message(), None, false)
            .respond(create_error_status(error)),
        Err(error) => {
            internal_error_response(format!("folder creation task failed: {error}"), None)
        }
    }
}

fn create_error_status(error: FolderCreateError) -> StatusCode {
    match error {
        FolderCreateError::InvalidName | FolderCreateError::OutsideRoot => StatusCode::BAD_REQUEST,
        FolderCreateError::MountNotFound | FolderCreateError::ParentNotFound => {
            StatusCode::NOT_FOUND
        }
        FolderCreateError::NameTaken | FolderCreateError::InsideVault => StatusCode::CONFLICT,
        FolderCreateError::NotWritable => StatusCode::UNPROCESSABLE_ENTITY,
    }
}

fn error_status(error: FolderListingError) -> StatusCode {
    match error {
        FolderListingError::OutsideRoot => StatusCode::BAD_REQUEST,
        FolderListingError::NotFound => StatusCode::NOT_FOUND,
        FolderListingError::Unreadable => StatusCode::UNPROCESSABLE_ENTITY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_creation_refusal_has_its_own_status_and_code() {
        use FolderCreateError::*;
        let cases = [
            (InvalidName, 400, "folder_name_invalid"),
            (OutsideRoot, 400, "folder_outside_root"),
            (MountNotFound, 404, "folder_mount_not_found"),
            (ParentNotFound, 404, "folder_parent_not_found"),
            (NameTaken, 409, "folder_name_taken"),
            (InsideVault, 409, "folder_inside_vault"),
            (NotWritable, 422, "folder_not_writable"),
        ];
        for (error, status, code) in cases {
            assert_eq!(create_error_status(error).as_u16(), status);
            assert_eq!(error.code(), code);
        }
    }

    #[test]
    fn each_refusal_has_its_own_status_and_code() {
        let cases = [
            (FolderListingError::OutsideRoot, 400, "folder_outside_root"),
            (FolderListingError::NotFound, 404, "folder_not_found"),
            (FolderListingError::Unreadable, 422, "folder_unreadable"),
        ];
        for (error, status, code) in cases {
            assert_eq!(error_status(error).as_u16(), status);
            assert_eq!(error.code(), code);
        }
    }
}
