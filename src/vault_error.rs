//! The transport-neutral structured error the Vault-qualified cores return.
//!
//! ADR-19 makes a core's typed outcome or its structured error
//! `{code, message, vault_id?, retryable}` the only thing an adapter sees.
//! The struct therefore cannot live in an adapter: it started life in
//! `handlers/vaults.rs` as `VaultApiError`, an HTTP name for a shape MCP
//! already re-serialised verbatim into a tool error. Here it belongs to
//! neither surface. The HTTP adapter still owns the mapping to a status code
//! (`VaultOperationError::respond`, which stays in `handlers/vaults.rs`
//! because it is axum-shaped), and the MCP adapter owns the mapping to a
//! JSON-RPC failure.
//!
//! `handlers::vaults::VaultApiError` remains as an alias for this type until
//! the Vault collection management ticket (#187) finishes moving the
//! collection handlers off it.

use serde::{Deserialize, Serialize};

use crate::vault_read::VaultReadError;
use crate::vault_registry::VaultId;

/// One failure from a Vault-qualified core, in the shape every surface
/// already reports: a stable machine-readable `code`, a human-readable
/// `message`, the Vault it concerns when the failure is Vault-qualified, and
/// whether retrying the same call could plausibly succeed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultOperationError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_id: Option<VaultId>,
    pub retryable: bool,
}

impl VaultOperationError {
    pub(crate) fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        vault_id: Option<VaultId>,
        retryable: bool,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            vault_id,
            retryable,
        }
    }
}

/// The read core's failures already carry this exact shape, so a mutation
/// that fails while resolving, gating, or indexing its Vault reports the same
/// `code` a read of that Vault would.
impl From<VaultReadError> for VaultOperationError {
    fn from(error: VaultReadError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            vault_id: error.vault_id,
            retryable: error.retryable,
        }
    }
}
