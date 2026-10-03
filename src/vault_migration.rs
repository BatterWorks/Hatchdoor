//! What is left of the legacy single-Vault import (ADR-40, #427).
//!
//! The import itself is gone: a deployment with no registry starts with an
//! empty one, whatever `VAULT_PATH` holds. Two startup steps outlive it. Every
//! start on an existing registry purges the retired Git-lane keys from stored
//! settings, so a plaintext Git token never survives there. And a start with
//! no registry but stored single-Vault keys refuses, because that is an
//! install from 2.4.x or earlier that never ran the import, and opening it on
//! zero Vaults would look like lost notes.

use std::fmt;
use std::fs;

use crate::runtime_config::{RuntimeConfig, SettingSource};
use crate::vault_registry::{VaultRegistryError, VaultRegistryState, VaultRegistryStore};

/// Where an install too old to upgrade directly is sent.
pub const LEGACY_MIGRATION_DOC_URL: &str =
    "https://github.com/BatterWorks/Hatchdoor/blob/main/docs/migrations/legacy-single-vault.md";

/// The retired instance-wide Git lane's inputs, the token among them. Nothing
/// reads them once a registry exists, so every start on one removes them
/// again (#325). For the same reason, any of them stored on an install
/// without a registry means a single-Vault release wrote them and the 2.5.0
/// to 2.7.x import never ran. `HATCHDOOR_EXCLUDE` and the two author keys are
/// neither purged nor evidence of an old install, because they are still live
/// settings a current install may store (the commit-identity fallback among
/// them).
const RETIRED_GIT_LANE_STORED_KEYS: [&str; 6] = [
    "HATCHDOOR_GIT_SYNC_ENABLED",
    "HATCHDOOR_GIT_HTTPS_TOKEN",
    "HATCHDOOR_GIT_REMOTE",
    "HATCHDOOR_GIT_BRANCH",
    "HATCHDOOR_GIT_HTTPS_USERNAME",
    "HATCHDOOR_GIT_DEBOUNCE_SECONDS",
];

/// Why startup could not settle the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryStartupError {
    /// No registry, but stored retired Git-lane settings only a single-Vault
    /// release wrote (ADR-40 decision 3). Nothing was written.
    UnconvertedLegacyInstall,
    Registry(VaultRegistryError),
    Storage(String),
}

impl fmt::Display for RegistryStartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnconvertedLegacyInstall => write!(
                formatter,
                "This install still has the single-Vault setup of Hatchdoor 2.4.x or earlier, \
                 which this version cannot convert. Upgrade to a 2.5.0 to 2.7.x release first and \
                 start it once, so it moves your Vault into the Vault registry, then upgrade to \
                 2.8.0 or later. Nothing was changed. See {LEGACY_MIGRATION_DOC_URL}"
            ),
            Self::Registry(error) => error.fmt(formatter),
            Self::Storage(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for RegistryStartupError {}

impl From<VaultRegistryError> for RegistryStartupError {
    fn from(error: VaultRegistryError) -> Self {
        Self::Registry(error)
    }
}

/// Load the registry for this start, writing an empty one when none exists.
///
/// Runs after the instance-state record (#424), which reads whether a
/// registry was already on disk, and before any Vault runtime starts.
pub fn prepare_registry(
    registry: &VaultRegistryStore,
    runtime_config: &RuntimeConfig,
) -> Result<VaultRegistryState, RegistryStartupError> {
    match fs::symlink_metadata(registry.path()) {
        Ok(_) => {
            if let Err(error) = runtime_config.remove_stored(RETIRED_GIT_LANE_STORED_KEYS) {
                tracing::warn!(
                    %error,
                    "could not remove retired Git settings; retrying on the next start"
                );
            }
            return Ok(registry.load()?);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(RegistryStartupError::Storage(format!(
                "could not inspect Vault registry path '{}': {error}",
                registry.path().display()
            )));
        }
    }

    let snapshot = runtime_config.snapshot();
    if RETIRED_GIT_LANE_STORED_KEYS.iter().any(|key| {
        snapshot
            .setting(key)
            .is_some_and(|setting| setting.source == SettingSource::Stored)
    }) {
        return Err(RegistryStartupError::UnconvertedLegacyInstall);
    }
    Ok(VaultRegistryState::Ready(registry.initialize_empty(0)?))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::tempdir;

    use super::{RegistryStartupError, prepare_registry};
    use crate::runtime_config::{Environment, RuntimeConfig, live_settings_defaults};
    use crate::vault_registry::{
        NewVaultDefinition, VaultRegistryState, VaultRegistryStore, VaultSource,
    };

    fn runtime_config(settings_path: &std::path::Path) -> RuntimeConfig {
        RuntimeConfig::load(
            settings_path,
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration")
    }

    fn directory_bytes(root: &std::path::Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
        walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .map(|entry| entry.expect("snapshot entry"))
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .expect("snapshot relative path")
                    .to_path_buf();
                let bytes = std::fs::read(entry.path()).expect("snapshot bytes");
                (relative, bytes)
            })
            .collect()
    }

    fn ready_vault_count(state: &VaultRegistryState) -> usize {
        match state {
            VaultRegistryState::Ready(snapshot) => snapshot.vault_ids().count(),
            VaultRegistryState::Recovery(recovery) => {
                panic!("registry in recovery: {}", recovery.message())
            }
        }
    }

    /// The stock Compose setup: Markdown already in the mounted folder, no
    /// registry yet. The folder is never registered and never written to.
    #[test]
    fn a_fresh_start_with_markdown_in_the_mount_writes_an_empty_registry() {
        let root = tempdir().expect("temporary deployment");
        let vault_path = root.path().join("vault");
        std::fs::create_dir_all(vault_path.join("Projects")).expect("vault folder");
        std::fs::write(vault_path.join("Welcome.md"), b"# Mine\n").expect("note");
        std::fs::write(vault_path.join("Projects/Plan.md"), b"[[Welcome]]\n").expect("note");
        let before = directory_bytes(&vault_path);
        let registry_path = root.path().join("state/vaults.json");
        let registry = VaultRegistryStore::new(&registry_path);

        let state = prepare_registry(
            &registry,
            &runtime_config(&root.path().join("settings.json")),
        )
        .expect("fresh start");

        assert_eq!(ready_vault_count(&state), 0);
        assert!(registry_path.exists(), "the empty registry is persisted");
        assert_eq!(ready_vault_count(&registry.load().expect("reload")), 0);
        assert_eq!(directory_bytes(&vault_path), before);
    }

    #[test]
    fn a_fresh_start_with_an_empty_mount_has_zero_vaults() {
        let root = tempdir().expect("temporary deployment");
        let vault_path = root.path().join("vault");
        std::fs::create_dir(&vault_path).expect("empty vault folder");
        let registry = VaultRegistryStore::new(root.path().join("state/vaults.json"));

        let state = prepare_registry(
            &registry,
            &runtime_config(&root.path().join("settings.json")),
        )
        .expect("fresh start");

        assert_eq!(ready_vault_count(&state), 0);
        assert_eq!(std::fs::read_dir(&vault_path).expect("read").count(), 0);
    }

    /// Live settings a current install may store, the exclusions and commit
    /// author among them, say nothing about the install's age, so they never
    /// trigger the refusal (a 2.8 install that lost its registry is not old).
    #[test]
    fn live_stored_settings_without_a_registry_still_start_empty() {
        let root = tempdir().expect("temporary deployment");
        let config = runtime_config(&root.path().join("settings.json"));
        config
            .save([
                (
                    "HATCHDOOR_ARCHIVE_PREFIX".to_string(),
                    "archive/".to_string(),
                ),
                ("HATCHDOOR_EXCLUDE".to_string(), "private/**".to_string()),
                (
                    "HATCHDOOR_GIT_AUTHOR_NAME".to_string(),
                    "Current Author".to_string(),
                ),
                (
                    "HATCHDOOR_GIT_AUTHOR_EMAIL".to_string(),
                    "author@example.com".to_string(),
                ),
            ])
            .expect("stored settings");
        let registry = VaultRegistryStore::new(root.path().join("state/vaults.json"));

        let state = prepare_registry(&registry, &config).expect("fresh start");

        assert_eq!(ready_vault_count(&state), 0);
    }

    #[test]
    fn stored_retired_git_lane_keys_without_a_registry_refuse_and_change_nothing() {
        for key in super::RETIRED_GIT_LANE_STORED_KEYS {
            let root = tempdir().expect("temporary deployment");
            let settings_path = root.path().join("settings.json");
            let config = runtime_config(&settings_path);
            config
                .save([(key.to_string(), "legacy-value".to_string())])
                .expect("stored legacy setting");
            let settings_before = std::fs::read(&settings_path).expect("settings file");
            let registry_path = root.path().join("state/vaults.json");

            let error =
                prepare_registry(&VaultRegistryStore::new(&registry_path), &config).expect_err(key);

            assert_eq!(
                error,
                RegistryStartupError::UnconvertedLegacyInstall,
                "{key}"
            );
            assert!(!registry_path.exists(), "{key}: no registry is written");
            assert_eq!(
                std::fs::read(&settings_path).expect("settings file"),
                settings_before,
                "{key}: stored settings are left for the 2.7.x import"
            );
        }
    }

    #[test]
    fn the_refusal_names_the_upgrade_path_and_the_migration_document() {
        let message = RegistryStartupError::UnconvertedLegacyInstall.to_string();

        assert!(message.contains("2.5.0 to 2.7.x"), "{message}");
        assert!(message.contains("2.8.0"), "{message}");
        assert!(
            message.contains(super::LEGACY_MIGRATION_DOC_URL),
            "{message}"
        );
    }

    #[test]
    fn an_existing_registry_purges_the_retired_git_lane_keys_and_keeps_the_rest() {
        let root = tempdir().expect("temporary deployment");
        let registry = VaultRegistryStore::new(root.path().join("state/vaults.json"));
        registry.initialize_empty(0).expect("existing registry");
        let settings_path = root.path().join("cache/settings.json");
        let config = runtime_config(&settings_path);
        config
            .save([
                (
                    "HATCHDOOR_GIT_HTTPS_TOKEN".to_string(),
                    "legacy-secret-token".to_string(),
                ),
                ("HATCHDOOR_GIT_SYNC_ENABLED".to_string(), "on".to_string()),
                (
                    "HATCHDOOR_GIT_AUTHOR_NAME".to_string(),
                    "Kept Author".to_string(),
                ),
                (
                    "HATCHDOOR_ARCHIVE_PREFIX".to_string(),
                    "archive/".to_string(),
                ),
            ])
            .expect("settings left behind by an interrupted cleanup");

        prepare_registry(&registry, &config).expect("existing registry");

        let stored = std::fs::read_to_string(&settings_path).expect("settings file");
        assert!(!stored.contains("legacy-secret-token"), "{stored}");
        assert!(!stored.contains("HATCHDOOR_GIT_SYNC_ENABLED"), "{stored}");
        let snapshot = runtime_config(&settings_path).snapshot();
        assert_eq!(
            snapshot
                .required("HATCHDOOR_GIT_AUTHOR_NAME")
                .expect("author"),
            "Kept Author"
        );
        assert_eq!(
            snapshot
                .required("HATCHDOOR_ARCHIVE_PREFIX")
                .expect("archive prefix"),
            "archive/"
        );
    }

    #[test]
    fn an_existing_registry_with_vaults_loads_exactly_as_before() {
        let root = tempdir().expect("temporary deployment");
        let vault_path = root.path().join("notes");
        std::fs::create_dir_all(&vault_path).expect("vault folder");
        std::fs::write(vault_path.join("Note.md"), b"# Note\n").expect("note");
        let registry_path = root.path().join("state/vaults.json");
        let registry = VaultRegistryStore::new(&registry_path);
        let committed = registry
            .add(
                0,
                NewVaultDefinition {
                    name: "Notes".to_string(),
                    enabled: true,
                    source: VaultSource::Local {
                        path: vault_path.clone(),
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect("registered Vault");
        let registry_before = std::fs::read(&registry_path).expect("registry bytes");
        let vault_before = directory_bytes(&vault_path);

        let state = prepare_registry(
            &registry,
            &runtime_config(&root.path().join("settings.json")),
        )
        .expect("existing registry");

        assert_eq!(state, VaultRegistryState::Ready(committed));
        assert_eq!(
            std::fs::read(&registry_path).expect("registry"),
            registry_before
        );
        assert_eq!(directory_bytes(&vault_path), vault_before);
    }

    /// An existing registry wins even when single-Vault keys are still stored:
    /// that install already ran the import, and only the retired Git-lane keys
    /// are purged.
    #[test]
    fn an_existing_registry_is_never_refused_for_stored_single_vault_keys() {
        let root = tempdir().expect("temporary deployment");
        let registry = VaultRegistryStore::new(root.path().join("state/vaults.json"));
        registry.initialize_empty(0).expect("existing registry");
        let config = runtime_config(&root.path().join("settings.json"));
        config
            .save([("HATCHDOOR_EXCLUDE".to_string(), "private/**".to_string())])
            .expect("stored exclusion");

        let state = prepare_registry(&registry, &config).expect("existing registry wins");

        assert_eq!(ready_vault_count(&state), 0);
    }
}
