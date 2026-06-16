use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;

use crate::app_state::AppState;

#[derive(Debug, Serialize)]
pub struct WriteCapabilitiesResponse {
    pub enabled: bool,
    pub warnings: Vec<String>,
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
