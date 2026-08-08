use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::app_state::AppState;

/// Upper bound on targets accepted by `/api/v1/vaults/{vault_id}/resolve-batch`
/// (DoS guard).
pub(crate) const MAX_RESOLVE_BATCH: usize = 200;

pub async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let _ = state;
    (StatusCode::OK, "ok").into_response()
}
