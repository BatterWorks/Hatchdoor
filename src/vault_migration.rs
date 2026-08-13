//! One-time import of proven legacy single-Vault deployments.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::parse_exclude_patterns;
use crate::git::config::{GitMode, non_empty_setting, parse_mode};
use crate::runtime_config::{RuntimeConfig, SettingSource};
use crate::vault_registry::{
    HttpsCredentials, NewVaultDefinition, VaultGitMode, VaultId, VaultRegistryError,
    VaultRegistrySnapshot, VaultRegistryState, VaultRegistryStore, VaultSource,
};

const LEGACY_STORED_KEYS: [&str; 9] = [
    "HATCHDOOR_EXCLUDE",
    "HATCHDOOR_GIT_SYNC_ENABLED",
    "HATCHDOOR_GIT_HTTPS_TOKEN",
    "HATCHDOOR_GIT_REMOTE",
    "HATCHDOOR_GIT_BRANCH",
    "HATCHDOOR_GIT_HTTPS_USERNAME",
    "HATCHDOOR_GIT_DEBOUNCE_SECONDS",
    "HATCHDOOR_GIT_AUTHOR_NAME",
    "HATCHDOOR_GIT_AUTHOR_EMAIL",
];

/// Legacy deployment inputs captured once before collection runtime starts.
#[derive(Clone, PartialEq, Eq)]
pub struct LegacyMigrationInput {
    pub vault_path: PathBuf,
    pub cache_db_path: PathBuf,
    pub environment: BTreeMap<String, String>,
}

impl fmt::Debug for LegacyMigrationInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyMigrationInput")
            .field("vault_path", &self.vault_path)
            .field("cache_db_path", &self.cache_db_path)
            .field(
                "environment_keys",
                &self.environment.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Observable result of inspecting and, when proven safe, importing legacy state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LegacyMigrationOutcome {
    NoLegacyDeployment,
    ExistingRegistry {
        state: VaultRegistryState,
        ignored_environment_keys: Vec<String>,
    },
    Imported {
        snapshot: VaultRegistrySnapshot,
        vault_id: VaultId,
        cleanup_warnings: Vec<String>,
        ignored_environment_keys: Vec<String>,
    },
    Recovery {
        recovery: LegacyMigrationRecovery,
        ignored_environment_keys: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyMigrationRecovery {
    message: String,
}

impl LegacyMigrationRecovery {
    pub const CODE: &'static str = "legacy_migration_required";

    pub fn code(&self) -> &'static str {
        Self::CODE
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LegacyMigrationError {
    ConfirmationRequired,
    Registry(VaultRegistryError),
    Storage(String),
}

impl fmt::Display for LegacyMigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfirmationRequired => formatter.write_str(
                "starting with no Vaults requires explicit confirmation because legacy state will no longer be imported",
            ),
            Self::Registry(error) => error.fmt(formatter),
            Self::Storage(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for LegacyMigrationError {}

impl From<VaultRegistryError> for LegacyMigrationError {
    fn from(error: VaultRegistryError) -> Self {
        Self::Registry(error)
    }
}

pub fn migrate_legacy_vault(
    registry: &VaultRegistryStore,
    runtime_config: &RuntimeConfig,
    input: LegacyMigrationInput,
) -> Result<LegacyMigrationOutcome, LegacyMigrationError> {
    let ignored_environment_keys = ignored_legacy_environment_keys(&input.environment);
    match fs::symlink_metadata(registry.path()) {
        Ok(_) => {
            return Ok(LegacyMigrationOutcome::ExistingRegistry {
                state: registry.load()?,
                ignored_environment_keys,
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(LegacyMigrationError::Storage(format!(
                "could not inspect Vault registry path '{}': {error}",
                registry.path().display()
            )));
        }
    }

    let snapshot = runtime_config.snapshot();
    let discard_legacy_cache = crate::cache::is_recognized_legacy_cache(&input.cache_db_path);
    let evidence = match legacy_evidence(&input, &snapshot, discard_legacy_cache) {
        Ok(evidence) => evidence,
        Err(message) => {
            return Ok(recovery(message, ignored_environment_keys));
        }
    };
    if !evidence {
        return Ok(LegacyMigrationOutcome::NoLegacyDeployment);
    }

    if !input.vault_path.is_dir() {
        return Ok(recovery(
            format!(
                "legacy Vault path '{}' is not a readable directory; correct VAULT_PATH or confirm Start with no Vaults",
                input.vault_path.display()
            ),
            ignored_environment_keys,
        ));
    }

    let (source, https_credentials) = match legacy_source(&input.vault_path, &snapshot) {
        Ok(source) => source,
        Err(message) => return Ok(recovery(message, ignored_environment_keys)),
    };
    let name = input
        .vault_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Migrated vault")
        .to_string();
    let exclude_patterns =
        parse_exclude_patterns(snapshot.required("HATCHDOOR_EXCLUDE").unwrap_or_default());
    let definition = NewVaultDefinition {
        name,
        enabled: true,
        source,
        exclude_patterns,
        https_credentials,
        archive_folder: None,
        commit_identity: None,
    };
    let imported = match registry.add(0, definition) {
        Ok(snapshot) => snapshot,
        Err(VaultRegistryError::InvalidDefinition(error)) => {
            return Ok(recovery(
                format!(
                    "legacy Vault configuration could not be converted safely: {error}; correct it or confirm Start with no Vaults"
                ),
                ignored_environment_keys,
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let vault_id = imported
        .vault_ids()
        .next()
        .expect("a successful legacy import creates exactly one Vault");

    let mut cleanup_warnings = Vec::new();
    if let Err(error) = runtime_config.remove_stored(LEGACY_STORED_KEYS) {
        cleanup_warnings.push(format!(
            "Vault registry committed, but migrated settings cleanup failed: {error}"
        ));
    }
    if discard_legacy_cache {
        for path in cache_files(&input.cache_db_path) {
            if let Err(error) = remove_if_present(&path) {
                cleanup_warnings.push(format!(
                    "Vault registry committed, but disposable cache cleanup failed for '{}': {error}",
                    path.display()
                ));
            }
        }
    }

    Ok(LegacyMigrationOutcome::Imported {
        snapshot: imported,
        vault_id,
        cleanup_warnings,
        ignored_environment_keys,
    })
}

fn legacy_source(
    vault_path: &Path,
    snapshot: &crate::runtime_config::ConfigSnapshot,
) -> Result<(VaultSource, Option<HttpsCredentials>), String> {
    for (key, default) in [
        ("HATCHDOOR_GIT_DEBOUNCE_SECONDS", "30"),
        ("HATCHDOOR_GIT_AUTHOR_NAME", "Hatchdoor"),
        ("HATCHDOOR_GIT_AUTHOR_EMAIL", "hatchdoor@localhost"),
    ] {
        if snapshot.setting(key).is_some_and(|setting| {
            setting.source != SettingSource::Default && setting.value != default
        }) {
            return Err(format!(
                "legacy setting {key} has no ordinary Vault-definition equivalent; restore its default, correct the deployment, or confirm Start with no Vaults"
            ));
        }
    }

    let mode = parse_mode(
        snapshot
            .required("HATCHDOOR_GIT_SYNC_ENABLED")
            .unwrap_or("false"),
    )
    .map_err(|error| format!("{error} Correct it or confirm Start with no Vaults"))?;
    match mode {
        None => Ok((
            VaultSource::Local {
                path: vault_path.to_path_buf(),
            },
            None,
        )),
        Some(GitMode::Local) => {
            open_exact_legacy_repository(vault_path)?;
            Ok((
                VaultSource::ExistingGit {
                    repository_path: vault_path.to_path_buf(),
                    repository_url: None,
                    branch: non_empty_setting(snapshot, "HATCHDOOR_GIT_BRANCH"),
                    vault_subdirectory: None,
                    mode: VaultGitMode::LocalHistory,
                },
                None,
            ))
        }
        Some(GitMode::Remote) => {
            let repository = open_exact_legacy_repository(vault_path)?;
            let remote_name = non_empty_setting(snapshot, "HATCHDOOR_GIT_REMOTE")
                .unwrap_or_else(|| "origin".to_string());
            let remote = repository.find_remote(&remote_name).map_err(|error| {
                format!(
                    "legacy Git remote '{remote_name}' could not be inspected: {error}; correct it or confirm Start with no Vaults"
                )
            })?;
            let repository_url = remote.url().map_err(|error| {
                format!(
                    "legacy Git remote '{remote_name}' has no readable URL: {error}; correct it or confirm Start with no Vaults"
                )
            })?;
            let token = non_empty_setting(snapshot, "HATCHDOOR_GIT_HTTPS_TOKEN");
            let https_credentials = token.map(|token| HttpsCredentials {
                username: non_empty_setting(snapshot, "HATCHDOOR_GIT_HTTPS_USERNAME")
                    .unwrap_or_else(|| "hatchdoor".to_string()),
                token,
            });
            Ok((
                VaultSource::ExistingGit {
                    repository_path: vault_path.to_path_buf(),
                    repository_url: Some(repository_url.to_string()),
                    branch: non_empty_setting(snapshot, "HATCHDOOR_GIT_BRANCH"),
                    vault_subdirectory: None,
                    mode: VaultGitMode::TwoWay,
                },
                https_credentials,
            ))
        }
    }
}

fn open_exact_legacy_repository(vault_path: &Path) -> Result<git2::Repository, String> {
    let repository = git2::Repository::open(vault_path).map_err(|error| {
        format!(
            "legacy Git Vault '{}' is not a readable repository: {error}; correct it or confirm Start with no Vaults",
            vault_path.display()
        )
    })?;
    if repository.is_bare() || repository.workdir().is_none() {
        return Err(
            "legacy Git Vault must be a non-bare working tree; correct it or confirm Start with no Vaults"
                .to_string(),
        );
    }
    Ok(repository)
}

fn recovery(
    message: impl Into<String>,
    ignored_environment_keys: Vec<String>,
) -> LegacyMigrationOutcome {
    LegacyMigrationOutcome::Recovery {
        recovery: LegacyMigrationRecovery {
            message: message.into(),
        },
        ignored_environment_keys,
    }
}

fn legacy_evidence(
    input: &LegacyMigrationInput,
    snapshot: &crate::runtime_config::ConfigSnapshot,
    recognized_cache: bool,
) -> Result<bool, String> {
    Ok(has_markdown(&input.vault_path)?
        || recognized_cache
        || has_git_history_with_explicit_config(&input.vault_path, snapshot)
        || LEGACY_STORED_KEYS.iter().any(|key| {
            snapshot
                .setting(key)
                .is_some_and(|setting| setting.source == SettingSource::Stored)
        })
        || deliberately_configured_vault_path(&input.environment))
}

fn has_git_history_with_explicit_config(
    vault_path: &Path,
    snapshot: &crate::runtime_config::ConfigSnapshot,
) -> bool {
    let explicit_git_config = LEGACY_STORED_KEYS
        .iter()
        .filter(|key| key.starts_with("HATCHDOOR_GIT_"))
        .any(|key| {
            snapshot
                .setting(key)
                .is_some_and(|setting| setting.source != SettingSource::Default)
        });
    if !explicit_git_config {
        return false;
    }
    git2::Repository::open(vault_path)
        .ok()
        .is_some_and(|repository| {
            !repository.is_bare()
                && repository.workdir().is_some()
                && repository
                    .head()
                    .ok()
                    .and_then(|head| head.target())
                    .is_some()
        })
}

fn has_markdown(vault_path: &Path) -> Result<bool, String> {
    if !vault_path.exists() {
        return Ok(false);
    }
    for entry in walkdir::WalkDir::new(vault_path)
        .follow_root_links(true)
        .follow_links(false)
    {
        let entry = entry.map_err(|error| {
            format!(
                "could not inspect legacy Vault '{}': {error}",
                vault_path.display()
            )
        })?;
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn deliberately_configured_vault_path(environment: &BTreeMap<String, String>) -> bool {
    let Some(value) = environment
        .get("VAULT_PATH")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let normalized = lexical_path(Path::new(value));
    normalized != Path::new("vault") && normalized != Path::new("/data/vault")
}

fn lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let can_pop = normalized
                    .file_name()
                    .is_some_and(|name| name != std::ffi::OsStr::new(".."));
                if can_pop {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn cache_files(path: &Path) -> [PathBuf; 3] {
    let mut wal: OsString = path.as_os_str().to_owned();
    wal.push("-wal");
    let mut shm: OsString = path.as_os_str().to_owned();
    shm.push("-shm");
    [path.to_path_buf(), PathBuf::from(wal), PathBuf::from(shm)]
}

fn remove_if_present(path: &Path) -> Result<(), std::io::Error> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn start_with_no_vaults(
    registry: &VaultRegistryStore,
    confirmed: bool,
) -> Result<crate::vault_registry::VaultRegistrySnapshot, LegacyMigrationError> {
    if !confirmed {
        return Err(LegacyMigrationError::ConfirmationRequired);
    }
    registry.initialize_empty(0).map_err(Into::into)
}

fn ignored_legacy_environment_keys(environment: &BTreeMap<String, String>) -> Vec<String> {
    environment
        .iter()
        .filter(|(key, value)| {
            !value.trim().is_empty()
                && (key.as_str() == "VAULT_PATH"
                    || key.as_str() == "HATCHDOOR_EXCLUDE"
                    || key.starts_with("HATCHDOOR_GIT_"))
        })
        .map(|(key, _)| key.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::tempdir;

    use super::{
        LegacyMigrationError, LegacyMigrationInput, LegacyMigrationOutcome, migrate_legacy_vault,
        start_with_no_vaults,
    };
    use crate::runtime_config::{Environment, RuntimeConfig, live_settings_defaults};
    use crate::vault_registry::{VaultRegistryState, VaultRegistryStore, VaultSource};

    fn write_recognized_legacy_cache(path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().expect("cache parent")).expect("cache directory");
        let connection = rusqlite::Connection::open(path).expect("legacy cache");
        connection
            .execute_batch(
                "CREATE TABLE metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);\n\
                 INSERT INTO metadata(key, value) VALUES ('schema_version', '8');\n\
                 CREATE TABLE notes(\n\
                    slug TEXT, title TEXT, normalized_title TEXT, relative_path TEXT,\n\
                    absolute_path TEXT, content TEXT, content_hash TEXT, indexed_at INTEGER\n\
                 );\n\
                 CREATE TABLE note_links(source_slug TEXT, target_slug TEXT);\n\
                 CREATE TABLE headings(note_slug TEXT);\n\
                 CREATE TABLE tags(note_slug TEXT);\n\
                 CREATE VIRTUAL TABLE note_fts USING fts5(title, relative_path, content, slug);",
            )
            .expect("legacy cache metadata");
    }

    fn write_recognized_v1_cache(path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().expect("cache parent")).expect("cache directory");
        let connection = rusqlite::Connection::open(path).expect("legacy v1 cache");
        connection
            .execute_batch(
                "CREATE TABLE metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);\n\
                 INSERT INTO metadata(key, value) VALUES ('schema_version', '1');\n\
                 CREATE TABLE notes(\n\
                    id INTEGER PRIMARY KEY, slug TEXT, title TEXT, relative_path TEXT,\n\
                    absolute_path TEXT, content TEXT, content_hash TEXT, indexed_at INTEGER\n\
                 );\n\
                 CREATE TABLE note_links(source_slug TEXT, target_slug TEXT);\n\
                 CREATE TABLE headings(note_slug TEXT);\n\
                 CREATE TABLE tags(note_slug TEXT);\n\
                 CREATE VIRTUAL TABLE note_fts USING fts5(title, relative_path, content, slug);",
            )
            .expect("legacy v1 schema");
    }

    fn initialize_legacy_repository(path: &std::path::Path) -> git2::Repository {
        std::fs::create_dir_all(path).expect("repository directory");
        std::fs::write(path.join("README.md"), b"# Existing Vault\n").expect("tracked Markdown");
        let repository = git2::Repository::init(path).expect("legacy repository");
        repository.set_head("refs/heads/main").expect("main branch");
        let mut index = repository.index().expect("index");
        index
            .add_path(std::path::Path::new("README.md"))
            .expect("stage note");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("tree");
        let tree = repository.find_tree(tree_id).expect("find tree");
        let signature =
            git2::Signature::now("Legacy User", "legacy@example.com").expect("signature");
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "legacy history",
                &tree,
                &[],
            )
            .expect("legacy commit");
        drop(tree);
        repository
    }

    fn commit_readme(repository: &git2::Repository, message: &str) -> git2::Oid {
        let mut index = repository.index().expect("index");
        index
            .add_path(std::path::Path::new("README.md"))
            .expect("stage README");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("tree");
        let tree = repository.find_tree(tree_id).expect("find tree");
        let parent_id = repository.head().expect("HEAD").target().expect("commit");
        let parent = repository.find_commit(parent_id).expect("parent");
        let signature =
            git2::Signature::now("Legacy User", "legacy@example.com").expect("signature");
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &[&parent],
            )
            .expect("commit")
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

    #[test]
    fn fresh_default_directory_does_not_trigger_migration() {
        let root = tempdir().expect("temporary deployment");
        let vault_path = root.path().join("vault");
        std::fs::create_dir(&vault_path).expect("empty default Vault directory");
        let registry_path = root.path().join("state/vaults.json");
        let settings_path = root.path().join("cache/settings.json");
        let runtime_config = RuntimeConfig::load(
            &settings_path,
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration");

        let outcome = migrate_legacy_vault(
            &VaultRegistryStore::new(&registry_path),
            &runtime_config,
            LegacyMigrationInput {
                vault_path,
                cache_db_path: root.path().join("cache/hatchdoor-cache.sqlite3"),
                environment: BTreeMap::new(),
            },
        )
        .expect("inspect fresh deployment");

        assert_eq!(outcome, LegacyMigrationOutcome::NoLegacyDeployment);
        assert!(!registry_path.exists());
        assert!(!settings_path.exists());
    }

    #[test]
    fn debug_output_lists_legacy_environment_keys_without_values() {
        let input = LegacyMigrationInput {
            vault_path: std::path::PathBuf::from("vault"),
            cache_db_path: std::path::PathBuf::from("cache.sqlite3"),
            environment: BTreeMap::from([(
                "HATCHDOOR_GIT_HTTPS_TOKEN".to_string(),
                "super-secret-token".to_string(),
            )]),
        };

        let debug = format!("{input:?}");
        assert!(debug.contains("HATCHDOOR_GIT_HTTPS_TOKEN"));
        assert!(!debug.contains("super-secret-token"));
    }

    #[test]
    fn lexically_equivalent_default_paths_do_not_prove_a_legacy_deployment() {
        for configured_path in [
            "vault/",
            "./vault",
            "notes/../vault",
            "/data/vault/",
            "/data/./vault",
            "/data/notes/../vault",
        ] {
            let root = tempdir().expect("temporary deployment");
            let vault_path = root.path().join("vault");
            std::fs::create_dir(&vault_path).expect("empty default Vault directory");
            let registry_path = root.path().join("state/vaults.json");
            let runtime_config = RuntimeConfig::load(
                root.path().join("cache/settings.json"),
                Environment::empty(),
                live_settings_defaults(),
            )
            .expect("runtime configuration");

            let outcome = migrate_legacy_vault(
                &VaultRegistryStore::new(&registry_path),
                &runtime_config,
                LegacyMigrationInput {
                    vault_path,
                    cache_db_path: root.path().join("cache/hatchdoor-cache.sqlite3"),
                    environment: BTreeMap::from([(
                        "VAULT_PATH".to_string(),
                        configured_path.to_string(),
                    )]),
                },
            )
            .expect("inspect equivalent default");

            assert_eq!(
                outcome,
                LegacyMigrationOutcome::NoLegacyDeployment,
                "{configured_path}"
            );
            assert!(!registry_path.exists(), "{configured_path}");
        }
    }

    #[test]
    fn any_existing_registry_permanently_suppresses_legacy_import() {
        let root = tempdir().expect("temporary deployment");
        let registry_path = root.path().join("state/vaults.json");
        let registry = VaultRegistryStore::new(&registry_path);
        registry
            .initialize_empty(0)
            .expect("intentional empty registry");
        let registry_bytes = std::fs::read(&registry_path).expect("registry bytes");
        let cache_db_path = root.path().join("cache/hatchdoor-cache.sqlite3");
        std::fs::create_dir_all(cache_db_path.parent().expect("cache parent"))
            .expect("cache directory");
        std::fs::write(&cache_db_path, b"legacy cache must remain").expect("legacy cache fixture");
        let runtime_config = RuntimeConfig::load(
            root.path().join("cache/settings.json"),
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration");

        let outcome = migrate_legacy_vault(
            &registry,
            &runtime_config,
            LegacyMigrationInput {
                vault_path: root.path().join("missing-deliberate-vault"),
                cache_db_path: cache_db_path.clone(),
                environment: BTreeMap::from([
                    (
                        "VAULT_PATH".to_string(),
                        root.path()
                            .join("missing-deliberate-vault")
                            .display()
                            .to_string(),
                    ),
                    ("HATCHDOOR_GIT_SYNC_ENABLED".to_string(), "true".to_string()),
                ]),
            },
        )
        .expect("existing registry wins");

        let LegacyMigrationOutcome::ExistingRegistry {
            state,
            ignored_environment_keys,
        } = outcome
        else {
            panic!("expected existing registry outcome");
        };
        assert!(matches!(state, VaultRegistryState::Ready(_)));
        assert_eq!(
            ignored_environment_keys,
            vec!["HATCHDOOR_GIT_SYNC_ENABLED", "VAULT_PATH"]
        );
        assert_eq!(
            std::fs::read(&registry_path).expect("registry retained"),
            registry_bytes
        );
        assert_eq!(
            std::fs::read(&cache_db_path).expect("cache retained"),
            b"legacy cache must remain"
        );
    }

    #[test]
    fn starting_with_no_vaults_requires_confirmation_and_persists_intent() {
        let root = tempdir().expect("temporary deployment");
        let registry_path = root.path().join("state/vaults.json");
        let registry = VaultRegistryStore::new(&registry_path);

        assert_eq!(
            start_with_no_vaults(&registry, false).expect_err("confirmation required"),
            LegacyMigrationError::ConfirmationRequired
        );
        assert!(!registry_path.exists());

        let snapshot = start_with_no_vaults(&registry, true).expect("confirmed empty registry");
        assert_eq!(snapshot.revision(), 1);
        assert_eq!(snapshot.vault_ids().len(), 0);
        assert!(registry_path.exists());
    }

    #[test]
    fn markdown_evidence_imports_local_vault_before_cleaning_disposable_state() {
        let root = tempdir().expect("temporary deployment");
        let vault_path = root.path().join("My Notes");
        std::fs::create_dir_all(vault_path.join("projects")).expect("Vault directory");
        let note_path = vault_path.join("projects/roadmap.md");
        std::fs::write(&note_path, b"# Roadmap\n\nKeep this authoritative.\n")
            .expect("legacy Markdown");
        let note_bytes = std::fs::read(&note_path).expect("note before migration");

        let cache_db_path = root.path().join("cache/hatchdoor-cache.sqlite3");
        write_recognized_legacy_cache(&cache_db_path);
        std::fs::write(cache_db_path.with_extension("sqlite3-wal"), b"stale wal")
            .expect("WAL fixture");
        std::fs::write(cache_db_path.with_extension("sqlite3-shm"), b"stale shm")
            .expect("SHM fixture");

        let settings_path = root.path().join("cache/settings.json");
        let runtime_config = RuntimeConfig::load(
            &settings_path,
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration");
        runtime_config
            .save([
                (
                    "HATCHDOOR_EXCLUDE".to_string(),
                    "private/**, tmp/**".to_string(),
                ),
                ("HATCHDOOR_GIT_SYNC_ENABLED".to_string(), "off".to_string()),
                (
                    "HATCHDOOR_ARCHIVE_PREFIX".to_string(),
                    "archive/".to_string(),
                ),
            ])
            .expect("legacy stored settings");

        let registry_path = root.path().join("state/vaults.json");
        let outcome = migrate_legacy_vault(
            &VaultRegistryStore::new(&registry_path),
            &runtime_config,
            LegacyMigrationInput {
                vault_path: vault_path.clone(),
                cache_db_path: cache_db_path.clone(),
                environment: BTreeMap::new(),
            },
        )
        .expect("safe legacy import");

        let LegacyMigrationOutcome::Imported {
            snapshot,
            vault_id,
            cleanup_warnings,
            ..
        } = outcome
        else {
            panic!("expected imported outcome");
        };
        assert!(cleanup_warnings.is_empty(), "{cleanup_warnings:?}");
        assert_eq!(snapshot.revision(), 1);
        let definition = snapshot.definition(vault_id).expect("imported definition");
        assert_eq!(definition.name(), "My Notes");
        assert!(definition.enabled());
        assert_eq!(
            definition.source(),
            &VaultSource::Local {
                path: vault_path.clone()
            }
        );
        assert_eq!(definition.exclude_patterns(), ["private/**", "tmp/**"]);
        assert_eq!(
            std::fs::read(&note_path).expect("note retained"),
            note_bytes
        );
        assert!(registry_path.exists());
        assert!(!cache_db_path.exists());
        assert!(!cache_db_path.with_extension("sqlite3-wal").exists());
        assert!(!cache_db_path.with_extension("sqlite3-shm").exists());

        let restarted = RuntimeConfig::load(
            &settings_path,
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration after cleanup");
        assert_eq!(
            restarted
                .snapshot()
                .required("HATCHDOOR_EXCLUDE")
                .expect("exclude"),
            ""
        );
        assert_eq!(
            restarted
                .snapshot()
                .required("HATCHDOOR_GIT_SYNC_ENABLED")
                .expect("git mode"),
            "false"
        );
        assert_eq!(
            restarted
                .snapshot()
                .required("HATCHDOOR_ARCHIVE_PREFIX")
                .expect("unrelated setting"),
            "archive/"
        );
    }

    #[test]
    fn local_git_mode_becomes_existing_local_history_without_touching_repository() {
        let root = tempdir().expect("temporary deployment");
        let vault_path = root.path().join("versioned-notes");
        let repository = initialize_legacy_repository(&vault_path);
        let head_before = repository.head().expect("HEAD").target().expect("commit");
        let config_before = std::fs::read(vault_path.join(".git/config")).expect("git config");
        let settings_path = root.path().join("cache/settings.json");
        let runtime_config = RuntimeConfig::load(
            &settings_path,
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration");
        runtime_config
            .save([(
                "HATCHDOOR_GIT_SYNC_ENABLED".to_string(),
                "local".to_string(),
            )])
            .expect("legacy local Git setting");

        let outcome = migrate_legacy_vault(
            &VaultRegistryStore::new(root.path().join("state/vaults.json")),
            &runtime_config,
            LegacyMigrationInput {
                vault_path: vault_path.clone(),
                cache_db_path: root.path().join("cache/hatchdoor-cache.sqlite3"),
                environment: BTreeMap::new(),
            },
        )
        .expect("local Git import");

        let LegacyMigrationOutcome::Imported {
            snapshot, vault_id, ..
        } = outcome
        else {
            panic!("expected imported outcome");
        };
        assert_eq!(
            snapshot.definition(vault_id).expect("definition").source(),
            &VaultSource::ExistingGit {
                repository_path: vault_path.clone(),
                repository_url: None,
                branch: Some("main".to_string()),
                vault_subdirectory: None,
                mode: crate::vault_registry::VaultGitMode::LocalHistory,
            }
        );
        let reopened = git2::Repository::open(&vault_path).expect("repository retained");
        assert_eq!(
            reopened.head().expect("HEAD retained").target(),
            Some(head_before)
        );
        assert_eq!(
            std::fs::read(vault_path.join(".git/config")).expect("git config retained"),
            config_before
        );
    }

    #[test]
    fn migration_preserves_dirty_branches_conflicts_commits_and_unpushed_work_byte_for_byte() {
        let root = tempdir().expect("temporary deployment");
        let vault_path = root.path().join("complex-git-state");
        let repository = initialize_legacy_repository(&vault_path);
        let initial_id = repository.head().expect("HEAD").target().expect("initial");
        let initial = repository.find_commit(initial_id).expect("initial commit");
        repository
            .branch("feature", &initial, false)
            .expect("additional branch");
        repository
            .reference(
                "refs/remotes/origin/main",
                initial_id,
                true,
                "legacy remote-tracking fixture",
            )
            .expect("remote-tracking ref");

        std::fs::write(vault_path.join("README.md"), b"# Local unpushed change\n")
            .expect("local change");
        let local_id = commit_readme(&repository, "local unpushed commit");

        let initial_tree = initial.tree().expect("initial tree");
        let remote_blob = repository
            .blob(b"# Conflicting remote change\n")
            .expect("remote blob");
        let mut remote_tree = repository
            .treebuilder(Some(&initial_tree))
            .expect("remote tree builder");
        remote_tree
            .insert("README.md", remote_blob, 0o100644)
            .expect("remote README");
        let remote_tree_id = remote_tree.write().expect("remote tree");
        drop(remote_tree);
        let remote_tree = repository
            .find_tree(remote_tree_id)
            .expect("find remote tree");
        let signature =
            git2::Signature::now("Remote User", "remote@example.com").expect("remote signature");
        let remote_id = repository
            .commit(
                None,
                &signature,
                &signature,
                "conflicting remote commit",
                &remote_tree,
                &[&initial],
            )
            .expect("remote commit");
        let annotated = repository
            .find_annotated_commit(remote_id)
            .expect("annotated remote commit");
        repository
            .merge(&[&annotated], None, None)
            .expect("leave genuine merge conflict");
        std::fs::write(vault_path.join("untracked-draft.md"), b"unsaved draft\n")
            .expect("untracked work");

        assert_eq!(repository.state(), git2::RepositoryState::Merge);
        assert!(
            repository
                .index()
                .expect("conflicted index")
                .has_conflicts()
        );
        assert_eq!(
            repository.head().expect("local HEAD").target(),
            Some(local_id)
        );
        let vault_before = directory_bytes(&vault_path);
        let runtime_config = RuntimeConfig::load(
            root.path().join("cache/settings.json"),
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration");
        runtime_config
            .save([(
                "HATCHDOOR_GIT_SYNC_ENABLED".to_string(),
                "local".to_string(),
            )])
            .expect("legacy local mode");

        let outcome = migrate_legacy_vault(
            &VaultRegistryStore::new(root.path().join("state/vaults.json")),
            &runtime_config,
            LegacyMigrationInput {
                vault_path: vault_path.clone(),
                cache_db_path: root.path().join("cache/hatchdoor-cache.sqlite3"),
                environment: BTreeMap::new(),
            },
        )
        .expect("conflicted repository import");

        assert!(matches!(outcome, LegacyMigrationOutcome::Imported { .. }));
        assert_eq!(directory_bytes(&vault_path), vault_before);
        let reopened = git2::Repository::open(&vault_path).expect("repository retained");
        assert_eq!(reopened.state(), git2::RepositoryState::Merge);
        assert!(
            reopened
                .index()
                .expect("conflicted index retained")
                .has_conflicts()
        );
        assert_eq!(
            reopened.head().expect("HEAD retained").target(),
            Some(local_id)
        );
        assert_eq!(
            reopened
                .refname_to_id("refs/heads/feature")
                .expect("feature branch retained"),
            initial_id
        );
        assert_eq!(
            reopened
                .refname_to_id("refs/remotes/origin/main")
                .expect("unpushed baseline retained"),
            initial_id
        );
    }

    #[test]
    fn remote_git_mode_maps_https_remote_branch_and_optional_credentials_read_only() {
        let root = tempdir().expect("temporary deployment");
        let vault_path = root.path().join("remote-notes");
        let repository = initialize_legacy_repository(&vault_path);
        repository
            .remote("upstream", "https://example.com/team/notes.git")
            .expect("legacy HTTPS remote");
        let head_before = repository.head().expect("HEAD").target().expect("commit");
        let config_before = std::fs::read(vault_path.join(".git/config")).expect("git config");
        let runtime_config = RuntimeConfig::load(
            root.path().join("cache/settings.json"),
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration");
        runtime_config
            .save([
                ("HATCHDOOR_GIT_SYNC_ENABLED".to_string(), "true".to_string()),
                ("HATCHDOOR_GIT_REMOTE".to_string(), "upstream".to_string()),
                ("HATCHDOOR_GIT_BRANCH".to_string(), "main".to_string()),
                (
                    "HATCHDOOR_GIT_HTTPS_USERNAME".to_string(),
                    "bot".to_string(),
                ),
                (
                    "HATCHDOOR_GIT_HTTPS_TOKEN".to_string(),
                    "secret".to_string(),
                ),
            ])
            .expect("legacy remote settings");

        let outcome = migrate_legacy_vault(
            &VaultRegistryStore::new(root.path().join("state/vaults.json")),
            &runtime_config,
            LegacyMigrationInput {
                vault_path: vault_path.clone(),
                cache_db_path: root.path().join("cache/hatchdoor-cache.sqlite3"),
                environment: BTreeMap::new(),
            },
        )
        .expect("remote Git import");

        let LegacyMigrationOutcome::Imported {
            snapshot, vault_id, ..
        } = outcome
        else {
            panic!("expected imported outcome");
        };
        let definition = snapshot.definition(vault_id).expect("definition");
        assert_eq!(
            definition.source(),
            &VaultSource::ExistingGit {
                repository_path: vault_path.clone(),
                repository_url: Some("https://example.com/team/notes.git".to_string()),
                branch: Some("main".to_string()),
                vault_subdirectory: None,
                mode: crate::vault_registry::VaultGitMode::TwoWay,
            }
        );
        assert!(definition.credential_configured());
        let reopened = git2::Repository::open(&vault_path).expect("repository retained");
        assert_eq!(
            reopened.head().expect("HEAD retained").target(),
            Some(head_before)
        );
        assert_eq!(
            std::fs::read(vault_path.join(".git/config")).expect("git config retained"),
            config_before
        );
    }

    #[test]
    fn recognized_cache_alone_proves_legacy_deployment_but_arbitrary_sqlite_does_not() {
        let root = tempdir().expect("temporary deployment");
        let vault_path = root.path().join("vault");
        std::fs::create_dir(&vault_path).expect("empty default Vault");
        let cache_db_path = root.path().join("cache/hatchdoor-cache.sqlite3");
        std::fs::create_dir_all(cache_db_path.parent().expect("cache parent"))
            .expect("cache directory");
        rusqlite::Connection::open(&cache_db_path)
            .expect("unrecognized SQLite file")
            .execute_batch(
                "CREATE TABLE metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);\n\
                 INSERT INTO metadata(key, value) VALUES ('schema_version', '8');\n\
                 CREATE TABLE unrelated(value TEXT);",
            )
            .expect("lookalike metadata fixture");
        let runtime_config = RuntimeConfig::load(
            root.path().join("cache/settings.json"),
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration");
        let registry_path = root.path().join("state/vaults.json");

        assert_eq!(
            migrate_legacy_vault(
                &VaultRegistryStore::new(&registry_path),
                &runtime_config,
                LegacyMigrationInput {
                    vault_path: vault_path.clone(),
                    cache_db_path: cache_db_path.clone(),
                    environment: BTreeMap::new(),
                },
            )
            .expect("inspect unrelated SQLite"),
            LegacyMigrationOutcome::NoLegacyDeployment
        );
        assert!(cache_db_path.exists());
        assert!(!registry_path.exists());

        std::fs::remove_file(&cache_db_path).expect("replace unrelated SQLite");
        write_recognized_legacy_cache(&cache_db_path);
        let outcome = migrate_legacy_vault(
            &VaultRegistryStore::new(&registry_path),
            &runtime_config,
            LegacyMigrationInput {
                vault_path,
                cache_db_path: cache_db_path.clone(),
                environment: BTreeMap::new(),
            },
        )
        .expect("recognized cache import");
        assert!(matches!(outcome, LegacyMigrationOutcome::Imported { .. }));
        assert!(registry_path.exists());
        assert!(!cache_db_path.exists());
    }

    #[test]
    fn genuine_schema_v1_cache_proves_an_empty_legacy_vault() {
        let root = tempdir().expect("temporary deployment");
        let vault_path = root.path().join("vault");
        std::fs::create_dir(&vault_path).expect("empty legacy Vault");
        let cache_db_path = root.path().join("cache/hatchdoor-cache.sqlite3");
        write_recognized_v1_cache(&cache_db_path);
        let runtime_config = RuntimeConfig::load(
            root.path().join("cache/settings.json"),
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration");

        let outcome = migrate_legacy_vault(
            &VaultRegistryStore::new(root.path().join("state/vaults.json")),
            &runtime_config,
            LegacyMigrationInput {
                vault_path,
                cache_db_path: cache_db_path.clone(),
                environment: BTreeMap::new(),
            },
        )
        .expect("schema v1 import");

        assert!(matches!(outcome, LegacyMigrationOutcome::Imported { .. }));
        assert!(!cache_db_path.exists());
    }

    #[test]
    fn markdown_import_never_deletes_an_unrecognized_cache_path() {
        let root = tempdir().expect("temporary deployment");
        let vault_path = root.path().join("vault");
        std::fs::create_dir(&vault_path).expect("Vault directory");
        std::fs::write(vault_path.join("note.md"), b"# Legacy note\n").expect("Markdown evidence");
        let cache_db_path = root.path().join("cache/hatchdoor-cache.sqlite3");
        std::fs::create_dir_all(cache_db_path.parent().expect("cache parent"))
            .expect("cache directory");
        std::fs::write(&cache_db_path, b"not a Hatchdoor cache").expect("unrecognized cache");
        let cache_before = std::fs::read(&cache_db_path).expect("cache bytes");
        let runtime_config = RuntimeConfig::load(
            root.path().join("cache/settings.json"),
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration");

        let outcome = migrate_legacy_vault(
            &VaultRegistryStore::new(root.path().join("state/vaults.json")),
            &runtime_config,
            LegacyMigrationInput {
                vault_path,
                cache_db_path: cache_db_path.clone(),
                environment: BTreeMap::new(),
            },
        )
        .expect("Markdown import");

        assert!(matches!(outcome, LegacyMigrationOutcome::Imported { .. }));
        assert_eq!(
            std::fs::read(cache_db_path).expect("unrecognized cache retained"),
            cache_before
        );
    }

    #[test]
    fn unsafe_conversion_enters_stable_recovery_without_mutating_legacy_state() {
        let root = tempdir().expect("temporary deployment");
        let vault_path = root.path().join("legacy-notes");
        std::fs::create_dir(&vault_path).expect("Vault directory");
        let note_path = vault_path.join("note.md");
        std::fs::write(&note_path, b"# Keep me\n").expect("legacy note");
        let cache_db_path = root.path().join("cache/hatchdoor-cache.sqlite3");
        write_recognized_legacy_cache(&cache_db_path);
        let settings_path = root.path().join("cache/settings.json");
        let runtime_config = RuntimeConfig::load(
            &settings_path,
            Environment::empty(),
            live_settings_defaults(),
        )
        .expect("runtime configuration");
        runtime_config
            .save([(
                "HATCHDOOR_GIT_AUTHOR_NAME".to_string(),
                "Custom Legacy Author".to_string(),
            )])
            .expect("unsupported legacy setting");
        let settings_before = std::fs::read(&settings_path).expect("settings bytes");
        let cache_before = std::fs::read(&cache_db_path).expect("cache bytes");
        let registry_path = root.path().join("state/vaults.json");

        let outcome = migrate_legacy_vault(
            &VaultRegistryStore::new(&registry_path),
            &runtime_config,
            LegacyMigrationInput {
                vault_path,
                cache_db_path: cache_db_path.clone(),
                environment: BTreeMap::new(),
            },
        )
        .expect("recovery outcome");
        let LegacyMigrationOutcome::Recovery { recovery, .. } = outcome else {
            panic!("expected recovery outcome");
        };
        assert_eq!(recovery.code(), "legacy_migration_required");
        assert!(recovery.message().contains("HATCHDOOR_GIT_AUTHOR_NAME"));
        assert!(!registry_path.exists());
        assert_eq!(
            std::fs::read(&note_path).expect("note retained"),
            b"# Keep me\n"
        );
        assert_eq!(
            std::fs::read(&cache_db_path).expect("cache retained"),
            cache_before
        );
        assert_eq!(
            std::fs::read(&settings_path).expect("settings retained"),
            settings_before
        );
    }
}
