//! HTTP redemption of transfer links (ADR-27).
//!
//! `GET /api/v1/vaults/{vault_id}/transfers/{*path}` downloads one attachment
//! and `POST` on the same path uploads one file, each on the strength of the
//! link's own signed query string rather than a bearer token. These routes sit
//! beside the asset and attachment routes instead of inside their guards, so
//! the bearer and web-token admission those guards perform is untouched.
//!
//! A link is only as good as the live configuration that honours it. Every
//! request re-reads the runtime snapshot: MCP disabled refuses every link, and
//! MCP write mode off refuses every upload link. Once admitted, a download runs
//! through the asset handler under the same byte ceiling and tool budget an
//! MCP-admitted asset read gets, and an upload through the same mutation core
//! the upload route uses, under `HATCHDOOR_MAX_ATTACHMENT_BYTES`.

use std::sync::Arc;

use axum::extract::{Extension, Multipart, Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::Response;

use crate::app_state::AppState;
use crate::auth::{McpAssetRead, too_many_requests};
use crate::handlers::vault_content::vault_scoped_asset_handler;
use crate::handlers::vault_write::{
    UploadForm, attachment_outcome_response, invalid_input_error, mutation_error_response,
    read_upload_form,
};
use crate::handlers::vaults::{VaultApiError, internal_error_response, parse_vault_id};
use crate::mcp::limits::RateLimiter;
use crate::mcp::subscriptions::McpBearerToken;
use crate::transfer_link::{LinkRefusal, SigningKey};
use crate::vault_mutation::VaultMutationCore;
use crate::vault_registry::VaultId;

/// `GET /api/v1/vaults/{vault_id}/transfers/{*path}` — redeem a download link.
pub async fn download_transfer_handler(
    State(state): State<AppState>,
    Extension(limiter): Extension<Arc<RateLimiter>>,
    Path((raw_vault_id, path)): Path<(String, String)>,
    uri: Uri,
) -> Response {
    let live = match LiveMcp::bind(&state) {
        Ok(live) => live,
        Err(response) => return *response,
    };
    let Ok(vault_id) = parse_vault_id(&raw_vault_id) else {
        return link_refusal(LinkRefusal::Invalid, None);
    };
    if let Err(refusal) =
        state
            .transfer_links
            .verify_download(&live.key, vault_id, &path, uri.query())
    {
        return link_refusal(refusal, Some(vault_id));
    }

    // The same budget an MCP-admitted asset read spends (`auth.rs`), against
    // the transport's own limiter, so a link is a cheaper transport and not a
    // second allowance.
    let guard = if live.rate_limits_enabled {
        match limiter.admit_tool_call(&live.token).await {
            Ok(guard) => Some(guard),
            Err(retry_in) => return too_many_requests(retry_in),
        }
    } else {
        None
    };

    let response = vault_scoped_asset_handler(
        State(state),
        Some(Extension(McpAssetRead {
            max_bytes: live.max_base64_bytes,
        })),
        Path((raw_vault_id, path)),
    )
    .await;
    drop(guard);
    response
}

/// `POST /api/v1/vaults/{vault_id}/transfers/{*path}` — redeem an upload link
/// with the upload route's own multipart form. The form's
/// `target_relative_path` may be left out; when sent, it must name the link's
/// own target.
pub async fn upload_transfer_handler(
    State(state): State<AppState>,
    Path((raw_vault_id, path)): Path<(String, String)>,
    uri: Uri,
    mut multipart: Multipart,
) -> Response {
    let live = match LiveMcp::bind(&state) {
        Ok(live) => live,
        Err(response) => return *response,
    };
    let Ok(vault_id) = parse_vault_id(&raw_vault_id) else {
        return link_refusal(LinkRefusal::Invalid, None);
    };
    if !live.write_enabled {
        return VaultApiError::new(
            "mcp_write_disabled",
            "Uploads through transfer links are off because MCP write mode is disabled.",
            Some(vault_id),
            false,
        )
        .respond(StatusCode::FORBIDDEN);
    }
    // Spent here, before the body is read: a link is good for one attempt.
    let overwrite =
        match state
            .transfer_links
            .redeem_upload(&live.key, vault_id, &path, uri.query())
        {
            Ok(overwrite) => overwrite,
            Err(refusal) => return link_refusal(refusal, Some(vault_id)),
        };

    let UploadForm {
        target_relative_path,
        file_bytes,
    } = match read_upload_form(&mut multipart, vault_id, live.max_attachment_bytes).await {
        Ok(form) => form,
        Err((status, error)) => return error.respond(status),
    };
    if let Some(named) = target_relative_path
        && named.trim() != path
    {
        return VaultApiError::new(
            LinkRefusal::Invalid.code(),
            format!("This upload link is for '{path}', not '{}'.", named.trim()),
            Some(vault_id),
            false,
        )
        .respond(StatusCode::FORBIDDEN);
    }
    let file_bytes = match file_bytes {
        Some(bytes) if !bytes.is_empty() => bytes,
        _ => {
            let (status, error) = invalid_input_error(vault_id, "file");
            return error.respond(status);
        }
    };

    match VaultMutationCore::from_state(&state)
        .import_attachment(
            vault_id,
            &path,
            file_bytes,
            live.max_attachment_bytes,
            overwrite,
        )
        .await
    {
        Ok(outcome) => attachment_outcome_response(vault_id, outcome),
        Err(error) => mutation_error_response(error),
    }
}

/// What one redemption binds from the live configuration: the limits it runs
/// under, and the key and budget of the MCP token current right now.
struct LiveMcp {
    key: SigningKey,
    token: McpBearerToken,
    write_enabled: bool,
    rate_limits_enabled: bool,
    max_base64_bytes: u64,
    max_attachment_bytes: u64,
}

impl LiveMcp {
    /// Refuses every link while MCP is off. A configuration that does not
    /// parse, or MCP enabled with no token, refuses too, rather than letting a
    /// malformed snapshot open the route.
    fn bind(state: &AppState) -> Result<Self, Box<Response>> {
        let mcp = AppState::runtime_mcp_config(&state.runtime_snapshot())
            .map_err(|error| Box::new(internal_error_response(error, None)))?;
        let token = match (mcp.enabled, mcp.bearer_token) {
            (true, Some(token)) => token,
            _ => {
                return Err(Box::new(
                    VaultApiError::new(
                        "mcp_disabled",
                        "Transfer links are off because MCP is disabled.",
                        None,
                        false,
                    )
                    .respond(StatusCode::FORBIDDEN),
                ));
            }
        };
        Ok(Self {
            key: state.transfer_links.key(&state.runtime_config, &token),
            token: McpBearerToken(Arc::from(token)),
            write_enabled: mcp.write_enabled,
            rate_limits_enabled: mcp.rate_limits_enabled,
            max_base64_bytes: mcp.max_base64_bytes,
            max_attachment_bytes: mcp.max_attachment_bytes,
        })
    }
}

fn link_refusal(refusal: LinkRefusal, vault_id: Option<VaultId>) -> Response {
    VaultApiError::new(refusal.code(), refusal.message(), vault_id, false)
        .respond(StatusCode::FORBIDDEN)
}
