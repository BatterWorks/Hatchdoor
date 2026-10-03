//! `GET /api/v1/folders`: the HTTP adapter over the folder listing (ADR-41).
//!
//! The listing walks the filesystem, so it runs on the blocking pool. The
//! route is mounted behind the web token and refused in demo mode by
//! `src/server.rs`; this adapter only parses the query and shapes the answer.

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::folder_listing::{FolderListingError, ListingLimits, list_folders};
use crate::handlers::vaults::{VaultApiError, internal_error_response, query_rejection_response};

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
