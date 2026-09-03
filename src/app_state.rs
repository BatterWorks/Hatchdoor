use std::sync::{Arc, RwLock as StdRwLock};

use axum::Json;
use axum::http::StatusCode;
use tokio::sync::broadcast;
use tracing::{error, info};

use crate::api_types::ErrorResponse;
use crate::cache::SqliteCache;
use crate::embed::Embedder;
use crate::startup::StartupTracker;
use crate::vault::VaultScanConfig;
use crate::vault_migration::LegacyMigrationRecovery;
use crate::vault_registry::{VaultDefinition, VaultRegistryStore};
use crate::vault_runtime::VaultCollectionRuntime;

#[derive(Clone)]
pub struct AppState {
    /// Authoritative Vault definitions and the independently activated runtime
    /// collection derived from them.
    pub vault_registry: VaultRegistryStore,
    pub vaults: VaultCollectionRuntime,
    /// The instance-wide background-work admission queue and its per-Vault
    /// managed-Git scheduler. HTTP Vault-collection management (add/edit/
    /// enable/disable/disconnect) reconciles through these after a registry
    /// commit; manual sync/retry request a Git turn directly.
    pub vault_work: crate::vault_work::VaultWorkCoordinator,
    pub managed_git: Arc<crate::git::ManagedGitScheduler>,
    /// Present when safe automatic import could not prove the legacy
    /// deployment. Collection/setup surfaces remain available for recovery.
    /// Cleared by a confirmed "Start with no Vaults"
    /// (`start_with_no_vaults_handler`), so this needs interior mutability
    /// rather than a plain `Option` fixed at startup.
    pub legacy_migration_recovery: Arc<StdRwLock<Option<LegacyMigrationRecovery>>>,
    /// The one SQLite database every Vault's snapshot is read from and
    /// written to. Opened at startup, before any Vault runtime is activated.
    pub startup_sqlite: Arc<SqliteCache>,
    /// Fires when the advertised MCP tool catalogue actually changes, which
    /// today means `HATCHDOOR_MCP_WRITE_ENABLED` was toggled: the write tools
    /// appear or disappear from `tools/list`. A subscribed streaming MCP
    /// session turns each signal into a `notifications/tools/list_changed`,
    /// backing the advertised `tools.listChanged` capability.
    ///
    /// A layer-marker change deliberately does *not* fire this: no tool schema
    /// is derived from the marker set, so re-listing would report no
    /// difference.
    pub mcp_tools_changed: broadcast::Sender<()>,
    pub embedder: Arc<dyn Embedder>,
    /// Concrete startup slot behind `embedder`; populated only after a model is
    /// selected and downloaded.
    pub runtime_embedder: Arc<crate::embed::RuntimeEmbedder>,
    pub model_setup: Arc<crate::model_setup::ModelSetup>,
    pub model_setup_started: Arc<std::sync::atomic::AtomicBool>,
    /// True when the web API is protected by `HATCHDOOR_WEB_BEARER_TOKEN`.
    pub web_auth_enabled: bool,
    /// True when public demo browsing is enabled and app-level writes are blocked.
    pub demo_mode: bool,
    /// Captured environment plus durable live-applicable settings. Consumers
    /// bind an immutable snapshot once per operation.
    pub runtime_config: crate::runtime_config::RuntimeConfig,
    pub startup: StartupTracker,
}

impl AppState {
    /// Bind the current live configuration once at an operation boundary.
    pub fn runtime_snapshot(&self) -> Arc<crate::runtime_config::ConfigSnapshot> {
        self.runtime_config.snapshot()
    }

    pub fn runtime_mcp_config(
        snapshot: &crate::runtime_config::ConfigSnapshot,
    ) -> Result<crate::mcp::McpConfig, String> {
        crate::mcp::McpConfig::from_snapshot(snapshot)
    }

    pub fn runtime_archive_prefix(
        snapshot: &crate::runtime_config::ConfigSnapshot,
    ) -> Result<Arc<str>, String> {
        snapshot
            .setting("HATCHDOOR_ARCHIVE_PREFIX")
            .map(|setting| Arc::from(setting.value.as_str()))
            .ok_or_else(|| "runtime configuration is missing HATCHDOOR_ARCHIVE_PREFIX".to_string())
    }

    /// Resolve the archive folder for one Vault: its own configured folder
    /// (`VaultDefinition::archive_folder`) if set, else the instance-wide
    /// `HATCHDOOR_ARCHIVE_PREFIX` default. `definition` is `None` when the
    /// Vault has no reconciled runtime yet — that degrades to the
    /// instance-wide default rather than failing, since the caller's own
    /// Vault-existence check (a not-found/unavailable status) is the
    /// authoritative error for that case.
    pub fn vault_archive_prefix(
        definition: Option<&VaultDefinition>,
        snapshot: &crate::runtime_config::ConfigSnapshot,
    ) -> Result<Arc<str>, String> {
        if let Some(folder) = definition.and_then(VaultDefinition::archive_folder) {
            return Ok(Arc::from(folder));
        }
        Self::runtime_archive_prefix(snapshot)
    }

    pub fn runtime_scan_config(
        snapshot: &crate::runtime_config::ConfigSnapshot,
    ) -> Result<Arc<VaultScanConfig>, String> {
        let patterns = snapshot
            .setting("HATCHDOOR_EXCLUDE")
            .map(|setting| crate::config::parse_exclude_patterns(&setting.value))
            .ok_or_else(|| "runtime configuration is missing HATCHDOOR_EXCLUDE".to_string())?;
        Ok(Arc::new(VaultScanConfig {
            exclude: crate::vault::ExcludeMatcher::new(&patterns)?,
        }))
    }
}

/// Build a generic `500` response, logging the real detail rather than leaking
/// it (absolute paths, internal error strings) to the client.
pub fn internal_error(detail: impl AsRef<str>) -> (StatusCode, Json<ErrorResponse>) {
    error!(detail = %detail.as_ref(), "Internal server error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal server error".to_string(),
        }),
    )
}

/// Run blocking (SQLite / embedding) work off the async runtime so it never
/// hogs a tokio worker or stalls other requests.
pub async fn run_blocking<T, F>(f: F) -> Result<T, (StatusCode, Json<ErrorResponse>)>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(internal_error(error)),
        Err(join_error) => Err(internal_error(format!(
            "background task panicked: {join_error}"
        ))),
    }
}

/// Request one Index turn for every currently active Vault after an indexing
/// setting has been persisted.
///
/// This replaced the instance-wide `schedule_settings_reindex` rebuild #185
/// deleted: instead of one rebuild it goes through the same admission
/// queue every other Index turn uses, so the turns stay globally serial (no
/// second execution lane, ADR-13) and each Vault's own condition reports
/// `indexing` while its turn runs. A disabled Vault has no active runtime and
/// is therefore never queued.
///
/// Returns the number of Vaults whose turn was accepted, for logging and
/// tests. A turn coalesced into work already pending for the same Vault is not
/// counted: the pending turn will read the newly persisted settings anyway,
/// because each turn binds the runtime snapshot current when it is dispatched.
pub fn request_collection_reindex(state: &AppState) -> usize {
    let queued = state
        .vaults
        .active_vault_ids()
        .into_iter()
        .filter(|vault_id| {
            matches!(
                state
                    .vault_work
                    .request(*vault_id, crate::vault_work::VaultWorkKind::Index),
                crate::vault_work::ScheduleResult::Queued
            )
        })
        .count();
    info!(
        vaults = queued,
        "Indexing settings changed; requested an Index turn per active Vault"
    );
    queued
}

#[cfg(test)]
pub fn test_embedder() -> Arc<dyn Embedder> {
    Arc::new(crate::embed::StubEmbedder::new(384))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Issue #130: a Vault's own configured archive folder overrides the
    /// instance-wide `HATCHDOOR_ARCHIVE_PREFIX` default when present, and the
    /// default applies unchanged both when the Vault has none configured and
    /// when no Vault definition is available at all (e.g. no reconciled
    /// runtime yet).
    #[test]
    fn vault_archive_prefix_prefers_the_vaults_own_folder_and_falls_back_to_the_instance_default() {
        let directory = tempdir().expect("temp dir");
        let vault_path = directory.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("vault dir");
        let store =
            crate::vault_registry::VaultRegistryStore::new(directory.path().join("vaults.json"));
        let committed = store
            .add(
                0,
                crate::vault_registry::NewVaultDefinition {
                    name: "Team Vault".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::Local {
                        path: vault_path.clone(),
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: Some("Team Archive".to_string()),
                    commit_identity: None,
                },
            )
            .expect("add Vault with its own archive folder");
        let with_override = committed.definitions().next().expect("definition");

        std::fs::create_dir_all(directory.path().join("plain")).expect("plain vault dir");
        let plain_committed = store
            .add(
                committed.revision(),
                crate::vault_registry::NewVaultDefinition {
                    name: "Plain Vault".to_string(),
                    enabled: true,
                    source: crate::vault_registry::VaultSource::Local {
                        path: directory.path().join("plain"),
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("add Vault without an archive folder");
        let without_override = plain_committed
            .definitions()
            .find(|definition| definition.name() == "Plain Vault")
            .expect("plain definition");

        let snapshot = crate::runtime_config::RuntimeConfig::for_tests().snapshot();

        assert_eq!(
            AppState::vault_archive_prefix(Some(&with_override), &snapshot)
                .expect("resolve override")
                .as_ref(),
            "Team Archive/"
        );
        assert_eq!(
            AppState::vault_archive_prefix(Some(&without_override), &snapshot)
                .expect("resolve default")
                .as_ref(),
            "90-archive/"
        );
        assert_eq!(
            AppState::vault_archive_prefix(None, &snapshot)
                .expect("resolve default with no definition")
                .as_ref(),
            "90-archive/"
        );
    }
}
