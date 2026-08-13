use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

use tempfile::tempdir;

use super::{
    DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS, DEFAULT_VAULT_REGISTRY_PATH,
    HTTPS_CREDENTIALS_USERNAME_PLACEHOLDER, HttpsCredentialUpdate, HttpsCredentials,
    NewVaultDefinition, REGISTRY_SCHEMA_VERSION, VaultCommitIdentity, VaultDefinitionEdit,
    VaultDefinitionError, VaultGitMode, VaultId, VaultRecord, VaultRegistryError,
    VaultRegistryRecoveryKind, VaultRegistryState, VaultRegistryStore, VaultSource,
};

#[test]
fn absent_registry_is_zero_vault_without_creating_a_file() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let store = VaultRegistryStore::new(&path);

    let state = store.load().expect("load absent registry");
    let VaultRegistryState::Ready(snapshot) = state else {
        panic!("absent registry entered recovery");
    };

    assert_eq!(snapshot.schema_version(), REGISTRY_SCHEMA_VERSION);
    assert_eq!(snapshot.revision(), 0);
    assert_eq!(snapshot.vault_ids().count(), 0);
    assert!(!path.exists(), "zero-Vault load created the registry");
}

#[test]
fn initialize_empty_registry_persists_intentional_zero_vault_state() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let store = VaultRegistryStore::new(&path);

    let snapshot = store
        .initialize_empty(0)
        .expect("persist intentional empty registry");

    assert_eq!(snapshot.revision(), 1);
    assert_eq!(snapshot.vault_ids().count(), 0);
    assert!(path.exists());
}

#[test]
fn default_store_uses_the_authoritative_instance_state_path() {
    assert_eq!(
        VaultRegistryStore::at_default_path().path(),
        std::path::Path::new(DEFAULT_VAULT_REGISTRY_PATH)
    );
    assert_eq!(DEFAULT_VAULT_REGISTRY_PATH, "/data/state/vaults.json");
}

#[test]
fn generated_vault_ids_are_canonical_random_uuids() {
    let id = VaultId::generate().expect("generate Vault ID");
    let encoded = id.to_string();

    assert_eq!(encoded.len(), 36);
    assert_eq!(&encoded[8..9], "-");
    assert_eq!(&encoded[13..14], "-");
    assert_eq!(&encoded[18..19], "-");
    assert_eq!(&encoded[23..24], "-");
    assert_eq!(&encoded[14..15], "4", "UUID version");
    assert!(matches!(&encoded[19..20], "8" | "9" | "a" | "b"));
    assert_eq!(VaultId::from_str(&encoded).expect("parse generated ID"), id);
}

#[test]
fn vault_ids_reject_noncanonical_or_invalid_values() {
    let uppercase = "018F47A0-7768-4D0C-8DA3-5AA28D1C31C7";
    let wrong_version = "018f47a0-7768-7d0c-8da3-5aa28d1c31c7";

    assert!(VaultId::from_str(uppercase).is_err());
    assert!(VaultId::from_str(wrong_version).is_err());
    assert!(VaultId::from_str("not-a-uuid").is_err());
}

#[test]
fn local_definition_add_round_trips_through_the_public_snapshot() {
    let directory = tempdir().expect("temporary directory");
    let vault_path = directory.path().join("notes");
    std::fs::create_dir(&vault_path).expect("create Vault directory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));

    let committed = store
        .add(
            0,
            NewVaultDefinition {
                name: "Personal".to_string(),
                enabled: true,
                source: VaultSource::Local {
                    path: vault_path.clone(),
                },
                exclude_patterns: vec![" private/** ".to_string(), "".to_string()],
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add local Vault");

    assert_eq!(committed.revision(), 1);
    let definition = committed
        .definitions()
        .next()
        .expect("committed definition");
    assert_eq!(definition.name(), "Personal");
    assert!(definition.enabled());
    assert_eq!(
        definition.source(),
        &VaultSource::Local {
            path: vault_path.canonicalize().expect("canonical Vault path")
        }
    );
    assert_eq!(definition.exclude_patterns(), &["private/**"]);
    assert!(!definition.credential_configured());

    let VaultRegistryState::Ready(restarted) = store.load().expect("reload registry") else {
        panic!("valid registry entered recovery");
    };
    assert_eq!(restarted.definitions().next(), Some(definition));
}

#[test]
fn vault_names_are_unique_without_regard_to_case() {
    let directory = tempdir().expect("temporary directory");
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    std::fs::create_dir(&first_path).expect("create first Vault");
    std::fs::create_dir(&second_path).expect("create second Vault");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    store
        .add(0, local_definition("Personal", first_path, true))
        .expect("add first Vault");

    let error = store
        .add(1, local_definition("personal", second_path, true))
        .expect_err("case-insensitive duplicate name accepted");

    assert_eq!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::DuplicateName)
    );
    let VaultRegistryState::Ready(snapshot) = store.load().expect("reload registry") else {
        panic!("valid registry entered recovery");
    };
    assert_eq!(snapshot.revision(), 1);
    assert_eq!(snapshot.definitions().count(), 1);
}

#[test]
fn vault_name_uniqueness_handles_case_mappings_that_expand() {
    let directory = tempdir().expect("temporary directory");
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    std::fs::create_dir(&first_path).expect("create first Vault");
    std::fs::create_dir(&second_path).expect("create second Vault");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    store
        .add(0, local_definition("Straße", first_path, true))
        .expect("add first Vault");

    let error = store
        .add(1, local_definition("STRASSE", second_path, true))
        .expect_err("caseless-equivalent name accepted");

    assert_eq!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::DuplicateName)
    );
}

#[test]
fn disabled_definitions_continue_reserving_their_canonical_paths() {
    let directory = tempdir().expect("temporary directory");
    let parent = directory.path().join("notes");
    let nested = parent.join("nested");
    std::fs::create_dir_all(&nested).expect("create nested Vault paths");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    store
        .add(0, local_definition("Paused", parent, false))
        .expect("add disabled Vault");

    let error = store
        .add(1, local_definition("Nested", nested, true))
        .expect_err("nested path accepted");

    assert_eq!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::PathOverlap)
    );
    let VaultRegistryState::Ready(snapshot) = store.load().expect("reload registry") else {
        panic!("valid registry entered recovery");
    };
    assert_eq!(snapshot.revision(), 1);
}

#[test]
fn managed_https_credentials_are_persisted_but_redacted_from_reads_and_debug() {
    let directory = tempdir().expect("temporary directory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let credentials = HttpsCredentials {
        username: "git-user".to_string(),
        token: "super-secret-token".to_string(),
    };
    assert!(!format!("{credentials:?}").contains("super-secret-token"));

    let committed = store
        .add(
            0,
            NewVaultDefinition {
                name: "Remote notes".to_string(),
                enabled: true,
                source: VaultSource::ManagedGit {
                    repository_url: "https://example.test/owner/notes.git".to_string(),
                    branch: Some("main".to_string()),
                    vault_subdirectory: Some(PathBuf::from("knowledge/notes")),
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: Some(credentials),
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add managed Vault");

    let definition = committed.definitions().next().expect("definition");
    assert!(definition.credential_configured());
    assert!(!format!("{definition:?}").contains("super-secret-token"));
    assert_eq!(
        definition.source(),
        &VaultSource::ManagedGit {
            repository_url: "https://example.test/owner/notes.git".to_string(),
            branch: Some("main".to_string()),
            vault_subdirectory: Some(PathBuf::from("knowledge/notes")),
            mode: VaultGitMode::PullOnly,
            poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
        }
    );
    let encoded = std::fs::read_to_string(store.path()).expect("persisted registry");
    assert!(encoded.contains("super-secret-token"));
    assert!(!format!("{committed:?}").contains("super-secret-token"));
}

#[test]
fn crate_private_https_credentials_accessor_returns_plaintext_only_for_configured_vaults() {
    let directory = tempdir().expect("temporary directory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let credentials = HttpsCredentials {
        username: "git-user".to_string(),
        token: "super-secret-token".to_string(),
    };
    let committed = store
        .add(
            0,
            NewVaultDefinition {
                name: "Remote notes".to_string(),
                enabled: true,
                source: VaultSource::ManagedGit {
                    repository_url: "https://example.test/owner/notes.git".to_string(),
                    branch: Some("main".to_string()),
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: Some(credentials.clone()),
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add managed Vault");
    let configured_id = committed
        .definitions()
        .next()
        .expect("definition")
        .vault_id();

    let local_path = directory.path().join("local");
    std::fs::create_dir(&local_path).expect("local Vault directory");
    let committed = store
        .add(
            committed.revision(),
            NewVaultDefinition {
                name: "No credentials".to_string(),
                enabled: true,
                source: VaultSource::Local { path: local_path },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add local Vault");
    let unconfigured_id = committed
        .definitions()
        .find(|definition| definition.name() == "No credentials")
        .expect("local definition")
        .vault_id();

    assert_eq!(
        store
            .https_credentials(configured_id)
            .expect("credentials lookup"),
        Some(credentials)
    );
    assert_eq!(
        store
            .https_credentials(unconfigured_id)
            .expect("credentials lookup"),
        None
    );
    assert_eq!(
        store
            .https_credentials(VaultId::generate().expect("random Vault ID"))
            .expect("credentials lookup for absent Vault"),
        None
    );
}

#[test]
fn existing_git_source_requires_a_readable_checkout_and_canonicalizes_its_location() {
    let directory = tempdir().expect("temporary directory");
    let repository_path = directory.path().join("repository");
    git2::Repository::init(&repository_path).expect("initialize repository");
    std::fs::create_dir(repository_path.join("notes")).expect("create Vault subdirectory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));

    let committed = store
        .add(
            0,
            NewVaultDefinition {
                name: "Existing checkout".to_string(),
                enabled: true,
                source: VaultSource::ExistingGit {
                    repository_path: repository_path.clone(),
                    repository_url: None,
                    branch: None,
                    vault_subdirectory: Some(PathBuf::from("notes")),
                    mode: VaultGitMode::LocalHistory,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add existing checkout");

    assert_eq!(
        committed.definitions().next().expect("definition").source(),
        &VaultSource::ExistingGit {
            repository_path: repository_path
                .canonicalize()
                .expect("canonical repository"),
            repository_url: None,
            branch: None,
            vault_subdirectory: Some(PathBuf::from("notes")),
            mode: VaultGitMode::LocalHistory,
            poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
        }
    );

    let not_repository = directory.path().join("ordinary-directory");
    std::fs::create_dir(&not_repository).expect("create ordinary directory");
    let error = store
        .add(
            1,
            NewVaultDefinition {
                name: "Not Git".to_string(),
                enabled: true,
                source: VaultSource::ExistingGit {
                    repository_path: not_repository,
                    repository_url: None,
                    branch: None,
                    vault_subdirectory: None,
                    mode: VaultGitMode::LocalHistory,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect_err("ordinary directory accepted as existing Git");
    assert!(matches!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
    ));
}

#[test]
fn existing_git_retains_remote_identity_while_git_mode_changes_normally() {
    let directory = tempdir().expect("temporary directory");
    let repository_path = directory.path().join("repository");
    git2::Repository::init(&repository_path).expect("initialize repository");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let repository_url = "https://example.test/notes.git".to_string();
    let added = store
        .add(
            0,
            NewVaultDefinition {
                name: "Local history".to_string(),
                enabled: true,
                source: VaultSource::ExistingGit {
                    repository_path: repository_path.clone(),
                    repository_url: Some(repository_url.clone()),
                    branch: Some("main".to_string()),
                    vault_subdirectory: None,
                    mode: VaultGitMode::LocalHistory,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("connect local history without discarding remote identity");
    let vault_id = added.definitions().next().expect("definition").vault_id();

    let edited = store
        .edit(
            1,
            vault_id,
            VaultDefinitionEdit {
                name: "Remote enabled".to_string(),
                source: VaultSource::ExistingGit {
                    repository_path,
                    repository_url: Some(repository_url),
                    branch: Some("main".to_string()),
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: HttpsCredentialUpdate::Keep,
                confirm_identity_change: false,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("change only Git behavior while enabled");

    assert!(matches!(
        edited.definitions().next().expect("definition").source(),
        VaultSource::ExistingGit {
            repository_url: Some(url),
            mode: VaultGitMode::PullOnly,
            ..
        } if url == "https://example.test/notes.git"
    ));
}

/// Mirrors `handlers/vaults.rs`'s
/// `editing_only_the_poll_interval_updates_the_live_scheduler_without_disturbing_backoff`,
/// which proves this property for `ManagedGit` end to end through the live
/// scheduler: `poll_interval_secs` must stay outside `same_source_identity`
/// for `ExistingGit` too (issue #132), so editing only the schedule on an
/// enabled, remote-backed `ExistingGit` Vault succeeds without
/// `confirm_identity_change` or requiring the Vault be disabled first — the
/// registry-level half of that property, at the `same_source_identity`
/// comparison itself rather than the scheduler it feeds.
#[test]
fn existing_git_poll_interval_stays_outside_identity_while_enabled() {
    let directory = tempdir().expect("temporary directory");
    let repository_path = directory.path().join("repository");
    git2::Repository::init(&repository_path).expect("initialize repository");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let repository_url = "https://example.test/notes.git".to_string();
    let added = store
        .add(
            0,
            NewVaultDefinition {
                name: "Remote checkout".to_string(),
                enabled: true,
                source: VaultSource::ExistingGit {
                    repository_path: repository_path.clone(),
                    repository_url: Some(repository_url.clone()),
                    branch: None,
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add remote-backed existing checkout");
    let vault_id = added.definitions().next().expect("definition").vault_id();

    let edited = store
        .edit(
            1,
            vault_id,
            VaultDefinitionEdit {
                name: "Remote checkout".to_string(),
                source: VaultSource::ExistingGit {
                    repository_path,
                    repository_url: Some(repository_url),
                    branch: None,
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS * 2,
                },
                exclude_patterns: Vec::new(),
                https_credentials: HttpsCredentialUpdate::Keep,
                confirm_identity_change: false,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect(
            "an interval-only edit on an enabled ExistingGit Vault must not be treated as an \
             identity change",
        );

    assert_eq!(
        edited
            .definitions()
            .next()
            .expect("definition")
            .source()
            .managed_git_poll_interval(),
        Some(std::time::Duration::from_secs(
            DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS * 2
        )),
        "the edit must have actually persisted the new interval"
    );
}

#[cfg(unix)]
#[test]
fn existing_git_symlink_subdirectory_cannot_bypass_canonical_overlap_validation() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("temporary directory");
    let repository_path = directory.path().join("repository");
    git2::Repository::init(&repository_path).expect("initialize repository");
    let actual = repository_path.join("actual");
    std::fs::create_dir(&actual).expect("create actual Vault path");
    symlink("actual", repository_path.join("alias")).expect("create in-repository symlink");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let added = store
        .add(
            0,
            NewVaultDefinition {
                name: "Existing checkout".to_string(),
                enabled: false,
                source: VaultSource::ExistingGit {
                    repository_path,
                    repository_url: None,
                    branch: None,
                    vault_subdirectory: Some(PathBuf::from("alias")),
                    mode: VaultGitMode::LocalHistory,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add existing checkout");
    assert!(matches!(
        added.definitions().next().expect("definition").source(),
        VaultSource::ExistingGit {
            vault_subdirectory: Some(path),
            ..
        } if path == std::path::Path::new("actual")
    ));

    let error = store
        .add(1, local_definition("Same files", actual, true))
        .expect_err("symlink alias bypassed path overlap");

    assert_eq!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::PathOverlap)
    );
}

#[test]
fn existing_git_source_rejects_the_repository_metadata_directory_as_its_location() {
    let directory = tempdir().expect("temporary directory");
    let repository_path = directory.path().join("repository");
    git2::Repository::init(&repository_path).expect("initialize repository");
    let path = directory.path().join("vaults.json");
    let store = VaultRegistryStore::new(&path);

    let error = store
        .add(
            0,
            NewVaultDefinition {
                name: "Metadata is not a Vault".to_string(),
                enabled: true,
                source: VaultSource::ExistingGit {
                    repository_path: repository_path.join(".git"),
                    repository_url: None,
                    branch: None,
                    vault_subdirectory: None,
                    mode: VaultGitMode::LocalHistory,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect_err("repository metadata accepted as checkout root");

    assert!(matches!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
    ));
    assert!(!path.exists());
}

#[test]
fn malformed_https_repository_urls_save_nothing() {
    for repository_url in [
        "https://:bad/notes.git",
        "https://example.test/%zz/notes.git",
    ] {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("vaults.json");
        let store = VaultRegistryStore::new(&path);

        let error = store
            .add(
                0,
                NewVaultDefinition {
                    name: "Malformed remote".to_string(),
                    enabled: true,
                    source: VaultSource::ManagedGit {
                        repository_url: repository_url.to_string(),
                        branch: None,
                        vault_subdirectory: None,
                        mode: VaultGitMode::PullOnly,
                        poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
                    archive_folder: None,
                    commit_identity: None,
                },
            )
            .expect_err("malformed HTTPS URL accepted");

        assert!(matches!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
        ));
        assert!(!path.exists());
    }
}

/// Issue #97's reopening finding 2: a per-Vault managed-Git poll interval
/// shorter than `MIN_MANAGED_GIT_POLL_INTERVAL_SECS` (mirroring
/// `git::managed_task::BACKOFF_MAX`) must be rejected rather than silently
/// accepted — a "normal" schedule tighter than the bound meant to throttle
/// repeated transient failures would make that bound meaningless.
#[test]
fn a_poll_interval_below_the_minimum_saves_nothing() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let store = VaultRegistryStore::new(&path);

    let error = store
        .add(
            0,
            NewVaultDefinition {
                name: "Too-frequent remote".to_string(),
                enabled: true,
                source: VaultSource::ManagedGit {
                    repository_url: "https://example.test/owner/notes.git".to_string(),
                    branch: None,
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: 1,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect_err("below-minimum poll interval accepted");

    assert!(matches!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
    ));
    assert!(!path.exists());
}

/// Issue #132: the same floor applies to a remote-backed `ExistingGit`
/// source (`PullOnly`/`TwoWay`) — it is scheduled by `ManagedGitScheduler`
/// exactly like `ManagedGit`, so it shares the same reasoning
/// `a_poll_interval_below_the_minimum_saves_nothing` documents.
/// `LocalHistory` has no remote to schedule, so it is exempt (see
/// `existing_git_local_history_poll_interval_is_not_floor_checked`).
#[test]
fn an_existing_git_poll_interval_below_the_minimum_saves_nothing() {
    let directory = tempdir().expect("temporary directory");
    let repository_path = directory.path().join("repository");
    git2::Repository::init(&repository_path).expect("initialize repository");
    let path = directory.path().join("vaults.json");
    let store = VaultRegistryStore::new(&path);

    let error = store
        .add(
            0,
            NewVaultDefinition {
                name: "Too-frequent checkout".to_string(),
                enabled: true,
                source: VaultSource::ExistingGit {
                    repository_path,
                    repository_url: Some("https://example.test/notes.git".to_string()),
                    branch: None,
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: 1,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect_err("below-minimum poll interval accepted");

    assert!(matches!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
    ));
    assert!(!path.exists());
}

/// `LocalHistory` has no remote and is never scheduled (see
/// `VaultSource::managed_git_poll_interval`), so its `poll_interval_secs` —
/// always present on the struct, never meaningful for this mode — is not
/// floor-checked.
#[test]
fn existing_git_local_history_poll_interval_is_not_floor_checked() {
    let directory = tempdir().expect("temporary directory");
    let repository_path = directory.path().join("repository");
    git2::Repository::init(&repository_path).expect("initialize repository");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));

    let committed = store
        .add(
            0,
            NewVaultDefinition {
                name: "Local history checkout".to_string(),
                enabled: true,
                source: VaultSource::ExistingGit {
                    repository_path,
                    repository_url: None,
                    branch: None,
                    vault_subdirectory: None,
                    mode: VaultGitMode::LocalHistory,
                    poll_interval_secs: 1,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("a LocalHistory Vault's unused poll interval is not floor-checked");

    assert_eq!(
        committed
            .definitions()
            .next()
            .expect("definition")
            .source()
            .managed_git_poll_interval(),
        None,
        "LocalHistory never schedules, regardless of the stored interval"
    );
}

/// Issue #97's reopening finding 2: adding `poll_interval_secs` to
/// `VaultSource::ManagedGit` must not require a `REGISTRY_SCHEMA_VERSION`
/// bump — a registry record written before this field existed (same schema
/// version 1, no `poll_interval_secs` key at all) must still load `Ready`,
/// with the field defaulting rather than the whole registry entering
/// recovery.
#[test]
fn a_managed_git_record_written_before_poll_interval_secs_existed_still_loads_with_the_default() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let original = br#"{
        "schema_version": 1,
        "revision": 1,
        "vaults": {
            "018f47a0-7768-4d0c-8da3-5aa28d1c31c7": {
                "name": "Pre-interval managed Vault",
                "enabled": true,
                "source": {
                    "type": "managed_git",
                    "repository_url": "https://example.test/owner/notes.git",
                    "branch": null,
                    "vault_subdirectory": null,
                    "mode": "pull_only"
                },
                "exclude_patterns": []
            }
        }
    }"#;
    std::fs::write(&path, original).expect("write pre-interval registry");
    let store = VaultRegistryStore::new(&path);

    let VaultRegistryState::Ready(snapshot) = store.load().expect("load registry") else {
        panic!("a schema-compatible pre-interval record entered recovery");
    };

    let definition = snapshot.definitions().next().expect("one definition");
    assert_eq!(
        definition.source().managed_git_poll_interval(),
        Some(std::time::Duration::from_secs(
            DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS
        ))
    );
}

/// Issue #132: mirrors
/// `a_managed_git_record_written_before_poll_interval_secs_existed_still_loads_with_the_default`
/// for `ExistingGit` — adding `poll_interval_secs` to that variant must not
/// require a `REGISTRY_SCHEMA_VERSION` bump either. `load()` validates a
/// stored source through `stored_source_is_valid`/`normalize_structural_source`
/// only (no `fs::canonicalize` or `git2::Repository::open`), so this needs no
/// real checkout on disk — only an already-absolute, already-normalized
/// `repository_path` string.
#[test]
fn an_existing_git_record_written_before_poll_interval_secs_existed_still_loads_with_the_default() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let original = br#"{
        "schema_version": 1,
        "revision": 1,
        "vaults": {
            "018f47a0-7768-4d0c-8da3-5aa28d1c31c7": {
                "name": "Pre-interval existing checkout",
                "enabled": true,
                "source": {
                    "type": "existing_git",
                    "repository_path": "/tmp/hatchdoor-test-existing-git-repo",
                    "repository_url": "https://example.test/owner/notes.git",
                    "branch": null,
                    "vault_subdirectory": null,
                    "mode": "pull_only"
                },
                "exclude_patterns": []
            }
        }
    }"#;
    std::fs::write(&path, original).expect("write pre-interval registry");
    let store = VaultRegistryStore::new(&path);

    let VaultRegistryState::Ready(snapshot) = store.load().expect("load registry") else {
        panic!("a schema-compatible pre-interval record entered recovery");
    };

    let definition = snapshot.definitions().next().expect("one definition");
    assert_eq!(
        definition.source().managed_git_poll_interval(),
        Some(std::time::Duration::from_secs(
            DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS
        ))
    );
}

/// Issue #130: `add` normalizes a per-Vault archive folder to a single
/// trailing slash regardless of how the caller spelled it, and normalizes a
/// commit identity's name/email by trimming whitespace, mirroring how
/// `exclude_patterns` and HTTPS credentials are already normalized.
#[test]
fn archive_folder_and_commit_identity_are_normalized_and_round_trip_through_add() {
    let directory = tempdir().expect("temporary directory");
    let vault_path = directory.path().join("notes");
    std::fs::create_dir(&vault_path).expect("create Vault directory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));

    let committed = store
        .add(
            0,
            NewVaultDefinition {
                archive_folder: Some("  /Team Archive/  ".to_string()),
                commit_identity: Some(VaultCommitIdentity {
                    name: "  Work Bot  ".to_string(),
                    email: "  work-bot@example.test  ".to_string(),
                }),
                ..local_definition("Team Vault", vault_path, true)
            },
        )
        .expect("add Vault with per-Vault archive folder and commit identity");

    let definition = committed.definitions().next().expect("definition");
    assert_eq!(definition.archive_folder(), Some("Team Archive/"));
    assert_eq!(
        definition.commit_identity(),
        Some(&VaultCommitIdentity {
            name: "Work Bot".to_string(),
            email: "work-bot@example.test".to_string(),
        })
    );
}

#[test]
fn archive_folder_rejects_an_empty_or_control_character_value() {
    let directory = tempdir().expect("temporary directory");
    let vault_path = directory.path().join("notes");
    std::fs::create_dir(&vault_path).expect("create Vault directory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));

    for invalid in ["   ", "///", "bad\u{0007}folder"] {
        let error = store
            .add(
                0,
                NewVaultDefinition {
                    archive_folder: Some(invalid.to_string()),
                    ..local_definition("Vault", vault_path.clone(), true)
                },
            )
            .expect_err("invalid archive folder accepted");
        assert_eq!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidArchiveFolder)
        );
    }
}

#[test]
fn commit_identity_requires_a_non_empty_name_and_email_together() {
    let directory = tempdir().expect("temporary directory");
    let vault_path = directory.path().join("notes");
    std::fs::create_dir(&vault_path).expect("create Vault directory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));

    for invalid in [
        VaultCommitIdentity {
            name: String::new(),
            email: "person@example.test".to_string(),
        },
        VaultCommitIdentity {
            name: "Person".to_string(),
            email: "   ".to_string(),
        },
    ] {
        let error = store
            .add(
                0,
                NewVaultDefinition {
                    commit_identity: Some(invalid),
                    ..local_definition("Vault", vault_path.clone(), true)
                },
            )
            .expect_err("invalid commit identity accepted");
        assert_eq!(
            error,
            VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidCommitIdentity)
        );
    }
}

/// Mirrors `a_managed_git_record_written_before_poll_interval_secs_existed_still_loads_with_the_default`:
/// a registry record written before `archive_folder`/`commit_identity`
/// existed (same schema version 1, neither key present) must still load
/// `Ready`, with both fields defaulting to absent rather than the whole
/// registry entering recovery.
#[test]
fn a_record_written_before_archive_folder_and_commit_identity_existed_still_loads_with_defaults() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let original = br#"{
        "schema_version": 1,
        "revision": 1,
        "vaults": {
            "018f47a0-7768-4d0c-8da3-5aa28d1c31c7": {
                "name": "Pre-identity Vault",
                "enabled": true,
                "source": {
                    "type": "local",
                    "path": "/tmp/pre-identity-vault"
                },
                "exclude_patterns": []
            }
        }
    }"#;
    std::fs::write(&path, original).expect("write pre-identity registry");
    let store = VaultRegistryStore::new(&path);

    let VaultRegistryState::Ready(snapshot) = store.load().expect("load registry") else {
        panic!("a schema-compatible pre-identity record entered recovery");
    };

    let definition = snapshot.definitions().next().expect("one definition");
    assert_eq!(definition.archive_folder(), None);
    assert_eq!(definition.commit_identity(), None);
}

/// Issue #130: HTTPS credentials accept a token alone. An absent or
/// empty-after-trim username is substituted with the documented fixed
/// placeholder rather than rejected, since `git2::Cred::userpass_plaintext`
/// requires a non-empty username but token-authenticating providers accept
/// any non-empty value there.
#[test]
fn https_credentials_accept_a_token_alone_and_substitute_the_documented_placeholder_username() {
    let directory = tempdir().expect("temporary directory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));

    let committed = store
        .add(
            0,
            NewVaultDefinition {
                https_credentials: Some(HttpsCredentials {
                    username: String::new(),
                    token: "token-only-secret".to_string(),
                }),
                ..managed_git_definition("Token-only Vault")
            },
        )
        .expect("token-alone credentials accepted");
    let vault_id = committed
        .definitions()
        .next()
        .expect("definition")
        .vault_id();

    let credentials = store
        .https_credentials(vault_id)
        .expect("credentials lookup")
        .expect("credentials configured");
    assert_eq!(credentials.username, HTTPS_CREDENTIALS_USERNAME_PLACEHOLDER);
    assert_eq!(credentials.token, "token-only-secret");
}

#[test]
fn https_credentials_reject_an_empty_token_even_with_a_username() {
    let directory = tempdir().expect("temporary directory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));

    let error = store
        .add(
            0,
            NewVaultDefinition {
                https_credentials: Some(HttpsCredentials {
                    username: "git-user".to_string(),
                    token: "   ".to_string(),
                }),
                ..managed_git_definition("Empty-token Vault")
            },
        )
        .expect_err("empty token accepted");

    assert!(matches!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
    ));
}

/// Issue #130: `edit` can set a previously-absent archive folder/commit
/// identity, and can clear a previously-set one back to absent (the
/// instance-wide default), since both fields are plain `Option`s resent in
/// full on every edit — unlike HTTPS credentials, there is no secret to
/// protect from "Keep versus clear" ambiguity here.
#[test]
fn edit_can_set_and_then_clear_archive_folder_and_commit_identity() {
    let directory = tempdir().expect("temporary directory");
    let vault_path = directory.path().join("notes");
    std::fs::create_dir(&vault_path).expect("create Vault directory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let committed = store
        .add(0, local_definition("Vault", vault_path.clone(), false))
        .expect("add disabled Vault");
    let vault_id = committed
        .definitions()
        .next()
        .expect("definition")
        .vault_id();

    let committed = store
        .edit(
            committed.revision(),
            vault_id,
            VaultDefinitionEdit {
                archive_folder: Some("Archive".to_string()),
                commit_identity: Some(VaultCommitIdentity {
                    name: "Person".to_string(),
                    email: "person@example.test".to_string(),
                }),
                ..local_edit("Vault", vault_path.clone(), false)
            },
        )
        .expect("set archive folder and commit identity");
    let definition = committed.definitions().next().expect("definition");
    assert_eq!(definition.archive_folder(), Some("Archive/"));
    assert!(definition.commit_identity().is_some());

    let committed = store
        .edit(
            committed.revision(),
            vault_id,
            local_edit("Vault", vault_path, false),
        )
        .expect("clear archive folder and commit identity");
    let definition = committed.definitions().next().expect("definition");
    assert_eq!(definition.archive_folder(), None);
    assert_eq!(definition.commit_identity(), None);
}

#[test]
fn identity_changes_require_a_disabled_definition_and_explicit_confirmation() {
    let directory = tempdir().expect("temporary directory");
    let original_path = directory.path().join("original");
    let replacement_path = directory.path().join("replacement");
    std::fs::create_dir(&original_path).expect("create original Vault");
    std::fs::create_dir(&replacement_path).expect("create replacement Vault");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let added = store
        .add(0, local_definition("Notes", original_path, true))
        .expect("add Vault");
    let vault_id = added.definitions().next().expect("definition").vault_id();

    let enabled_error = store
        .edit(
            1,
            vault_id,
            local_edit("Notes", replacement_path.clone(), true),
        )
        .expect_err("enabled identity change accepted");
    assert_eq!(
        enabled_error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::IdentityChangeRequiresDisabled)
    );

    let disabled = store.disable(1, vault_id).expect("disable Vault");
    assert_eq!(disabled.revision(), 2);
    assert!(!disabled.definitions().next().expect("definition").enabled());

    let confirmation_error = store
        .edit(
            2,
            vault_id,
            local_edit("Notes", replacement_path.clone(), false),
        )
        .expect_err("unconfirmed identity change accepted");
    assert_eq!(
        confirmation_error,
        VaultRegistryError::InvalidDefinition(
            VaultDefinitionError::IdentityChangeRequiresConfirmation
        )
    );

    let edited = store
        .edit(
            2,
            vault_id,
            local_edit("Notes", replacement_path.clone(), true),
        )
        .expect("confirmed disabled identity change");
    let definition = edited.definitions().next().expect("definition");
    assert_eq!(definition.vault_id(), vault_id);
    assert!(!definition.enabled());
    assert_eq!(
        definition.source(),
        &VaultSource::Local {
            path: replacement_path
                .canonicalize()
                .expect("canonical replacement")
        }
    );
}

#[test]
fn disconnect_removes_only_the_definition_and_preserves_vault_files() {
    let directory = tempdir().expect("temporary directory");
    let vault_path = directory.path().join("notes");
    std::fs::create_dir(&vault_path).expect("create Vault");
    let note_path = vault_path.join("kept.md");
    std::fs::write(&note_path, "# Kept\n").expect("write note");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let added = store
        .add(0, local_definition("Notes", vault_path.clone(), true))
        .expect("add Vault");
    let vault_id = added.definitions().next().expect("definition").vault_id();

    let disconnected = store.disconnect(1, vault_id).expect("disconnect Vault");

    assert_eq!(disconnected.revision(), 2);
    assert_eq!(disconnected.definitions().count(), 0);
    assert_eq!(
        std::fs::read_to_string(note_path).expect("preserved note"),
        "# Kept\n"
    );
    assert!(vault_path.exists());
}

#[test]
fn structurally_invalid_persisted_definitions_enter_recovery_without_overwrite() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let original = br#"{
          "schema_version": 1,
          "revision": 8,
          "vaults": {
            "018f47a0-7768-4d0c-8da3-5aa28d1c31c7": {
              "name": "Personal",
              "enabled": true,
              "source": {"type": "local", "path": "/srv/first"},
              "exclude_patterns": []
            },
            "b676d7c4-ca1c-4c92-813f-b47b14a5192d": {
              "name": "personal",
              "enabled": false,
              "source": {"type": "local", "path": "/srv/second"},
              "exclude_patterns": []
            }
          }
        }"#;
    std::fs::write(&path, original).expect("write invalid registry");
    let store = VaultRegistryStore::new(&path);

    let VaultRegistryState::Recovery(recovery) = store.load().expect("load registry") else {
        panic!("invalid definitions were treated as ready");
    };

    assert_eq!(recovery.kind(), VaultRegistryRecoveryKind::Corrupt);
    assert_eq!(std::fs::read(&path).expect("preserved registry"), original);
    assert_eq!(
        store
            .disconnect(
                8,
                VaultId::from_str("018f47a0-7768-4d0c-8da3-5aa28d1c31c7").expect("known ID")
            )
            .expect_err("invalid registry overwritten"),
        VaultRegistryError::RecoveryRequired
    );
}

#[test]
fn credential_bearing_repository_urls_enter_redacted_recovery() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let original = br#"{
          "schema_version": 1,
          "revision": 4,
          "vaults": {
            "018f47a0-7768-4d0c-8da3-5aa28d1c31c7": {
              "name": "Remote",
              "enabled": true,
              "source": {
                "type": "managed_git",
                "repository_url": "https://git-user:url-secret@example.test/notes.git",
                "branch": "main",
                "vault_subdirectory": null,
                "mode": "pull_only"
              },
              "exclude_patterns": []
            }
          }
        }"#;
    std::fs::write(&path, original).expect("write unsafe registry");
    let store = VaultRegistryStore::new(&path);

    let VaultRegistryState::Recovery(recovery) = store.load().expect("load registry") else {
        panic!("credential-bearing URL was treated as ready");
    };

    assert_eq!(recovery.kind(), VaultRegistryRecoveryKind::Corrupt);
    assert!(!recovery.message().contains("url-secret"));
    assert_eq!(std::fs::read(&path).expect("preserved registry"), original);
}

#[test]
fn malformed_persisted_secret_fields_do_not_echo_values_in_recovery() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let original = br#"{
          "schema_version": 1,
          "revision": 4,
          "vaults": {
            "018f47a0-7768-4d0c-8da3-5aa28d1c31c7": {
              "name": "Remote",
              "enabled": true,
              "source": {
                "type": "managed_git",
                "repository_url": "https://example.test/notes.git",
                "branch": "main",
                "vault_subdirectory": null,
                "mode": "value-that-must-not-leak"
              },
              "exclude_patterns": []
            }
          }
        }"#;
    std::fs::write(&path, original).expect("write malformed registry");
    let store = VaultRegistryStore::new(&path);

    let VaultRegistryState::Recovery(recovery) = store.load().expect("load registry") else {
        panic!("malformed definition was treated as ready");
    };

    assert_eq!(recovery.kind(), VaultRegistryRecoveryKind::Corrupt);
    assert!(!recovery.message().contains("value-that-must-not-leak"));
    assert_eq!(std::fs::read(&path).expect("preserved registry"), original);
}

#[test]
fn enabled_definitions_allow_name_mode_and_credential_changes() {
    let directory = tempdir().expect("temporary directory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let source = VaultSource::ManagedGit {
        repository_url: "https://example.test/notes.git".to_string(),
        branch: Some("main".to_string()),
        vault_subdirectory: None,
        mode: VaultGitMode::PullOnly,
        poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
    };
    let added = store
        .add(
            0,
            NewVaultDefinition {
                name: "Remote".to_string(),
                enabled: true,
                source: source.clone(),
                exclude_patterns: Vec::new(),
                https_credentials: Some(HttpsCredentials {
                    username: "old-user".to_string(),
                    token: "old-token".to_string(),
                }),
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add remote Vault");
    let vault_id = added.definitions().next().expect("definition").vault_id();

    let edited = store
        .edit(
            1,
            vault_id,
            VaultDefinitionEdit {
                name: "Renamed remote".to_string(),
                source: VaultSource::ManagedGit {
                    repository_url: "https://example.test/notes.git".to_string(),
                    branch: Some("main".to_string()),
                    vault_subdirectory: None,
                    mode: VaultGitMode::TwoWay,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: vec!["drafts/**".to_string()],
                https_credentials: HttpsCredentialUpdate::Replace(HttpsCredentials {
                    username: "new-user".to_string(),
                    token: "new-token".to_string(),
                }),
                confirm_identity_change: false,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("edit non-identity fields");

    let definition = edited.definitions().next().expect("definition");
    assert!(definition.enabled());
    assert_eq!(definition.name(), "Renamed remote");
    assert!(definition.credential_configured());
    assert!(matches!(
        definition.source(),
        VaultSource::ManagedGit {
            mode: VaultGitMode::TwoWay,
            ..
        }
    ));
    let encoded = std::fs::read_to_string(store.path()).expect("registry");
    assert!(!encoded.contains("old-token"));
    assert!(encoded.contains("new-token"));

    let removed = store
        .edit(
            2,
            vault_id,
            VaultDefinitionEdit {
                name: "Renamed remote".to_string(),
                source: definition.source().clone(),
                exclude_patterns: vec!["drafts/**".to_string()],
                https_credentials: HttpsCredentialUpdate::Remove,
                confirm_identity_change: false,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("remove credential");
    assert!(
        !removed
            .definitions()
            .next()
            .expect("definition")
            .credential_configured()
    );
    assert!(
        !std::fs::read_to_string(store.path())
            .expect("registry")
            .contains("new-token")
    );
}

#[test]
fn non_identity_edits_do_not_require_the_source_to_be_currently_available() {
    let directory = tempdir().expect("temporary directory");
    let vault_path = directory.path().join("notes");
    std::fs::create_dir(&vault_path).expect("create Vault");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let added = store
        .add(
            0,
            local_definition("Mounted notes", vault_path.clone(), true),
        )
        .expect("add local Vault");
    let vault_id = added.definitions().next().expect("definition").vault_id();
    std::fs::remove_dir(&vault_path).expect("make source temporarily unavailable");

    let edited = store
        .edit(
            1,
            vault_id,
            local_edit("Renamed while unavailable", vault_path, false),
        )
        .expect("rename without changing source identity");

    assert_eq!(edited.revision(), 2);
    assert_eq!(
        edited.definitions().next().expect("definition").name(),
        "Renamed while unavailable"
    );
}

#[test]
fn invalid_sources_reject_the_transaction_without_leaking_credentials() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let store = VaultRegistryStore::new(&path);
    let unsafe_source = VaultSource::ManagedGit {
        repository_url: "https://git-user:embedded-secret@example.test/notes.git".to_string(),
        branch: None,
        vault_subdirectory: None,
        mode: VaultGitMode::PullOnly,
        poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
    };
    assert!(!format!("{unsafe_source:?}").contains("embedded-secret"));
    let error = store
        .add(
            0,
            NewVaultDefinition {
                name: "Unsafe remote".to_string(),
                enabled: true,
                source: unsafe_source,
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect_err("credential-bearing URL accepted");
    assert!(!error.to_string().contains("embedded-secret"));
    assert!(!path.exists());

    let error = store
        .add(
            0,
            NewVaultDefinition {
                name: "Escaping remote".to_string(),
                enabled: true,
                source: VaultSource::ManagedGit {
                    repository_url: "https://example.test/notes.git".to_string(),
                    branch: None,
                    vault_subdirectory: Some(PathBuf::from("../outside")),
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect_err("escaping subdirectory accepted");
    assert!(matches!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
    ));
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn readable_non_writable_local_vaults_remain_valid() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("temporary directory");
    let vault_path = directory.path().join("read-only-notes");
    std::fs::create_dir(&vault_path).expect("create Vault");
    std::fs::set_permissions(&vault_path, std::fs::Permissions::from_mode(0o555))
        .expect("make Vault non-writable");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));

    let committed = store
        .add(0, local_definition("Read only", vault_path, true))
        .expect("connect readable non-writable Vault");

    assert!(
        committed
            .definitions()
            .next()
            .expect("definition")
            .enabled()
    );
}

#[test]
fn enable_preserves_identity_and_increments_the_registry_revision() {
    let directory = tempdir().expect("temporary directory");
    let vault_path = directory.path().join("notes");
    std::fs::create_dir(&vault_path).expect("create Vault");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let added = store
        .add(0, local_definition("Paused", vault_path, false))
        .expect("add disabled Vault");
    let vault_id = added.definitions().next().expect("definition").vault_id();

    let enabled = store.enable(1, vault_id).expect("enable Vault");

    let definition = enabled.definitions().next().expect("definition");
    assert_eq!(enabled.revision(), 2);
    assert_eq!(definition.vault_id(), vault_id);
    assert!(definition.enabled());
}

#[test]
fn enable_revalidates_a_local_source_before_saving() {
    let directory = tempdir().expect("temporary directory");
    let vault_path = directory.path().join("notes");
    std::fs::create_dir(&vault_path).expect("create Vault");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let added = store
        .add(0, local_definition("Paused", vault_path.clone(), false))
        .expect("add disabled Vault");
    let vault_id = added.definitions().next().expect("definition").vault_id();
    std::fs::remove_dir(&vault_path).expect("make source unavailable");

    let error = store
        .enable(1, vault_id)
        .expect_err("unreadable source was enabled");

    assert!(matches!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::InvalidSource(_))
    ));
    let VaultRegistryState::Ready(snapshot) = store.load().expect("reload registry") else {
        panic!("valid registry entered recovery");
    };
    assert_eq!(snapshot.revision(), 1);
    assert!(!snapshot.definition(vault_id).expect("definition").enabled());
}

#[test]
fn confirmed_repository_change_does_not_carry_an_old_credential() {
    let directory = tempdir().expect("temporary directory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let added = store
        .add(
            0,
            NewVaultDefinition {
                name: "Paused remote".to_string(),
                enabled: false,
                source: VaultSource::ManagedGit {
                    repository_url: "https://old.example.test/notes.git".to_string(),
                    branch: Some("main".to_string()),
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: Some(HttpsCredentials {
                    username: "old-user".to_string(),
                    token: "old-repository-token".to_string(),
                }),
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add managed Vault");
    let vault_id = added.definitions().next().expect("definition").vault_id();

    let edited = store
        .edit(
            1,
            vault_id,
            VaultDefinitionEdit {
                name: "Paused remote".to_string(),
                source: VaultSource::ManagedGit {
                    repository_url: "https://new.example.test/notes.git".to_string(),
                    branch: Some("main".to_string()),
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: HttpsCredentialUpdate::Keep,
                confirm_identity_change: true,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("confirm repository change");

    assert!(
        !edited
            .definitions()
            .next()
            .expect("definition")
            .credential_configured()
    );
    assert!(
        !std::fs::read_to_string(store.path())
            .expect("registry")
            .contains("old-repository-token")
    );
}

#[test]
fn managed_checkout_paths_participate_in_overlap_validation() {
    let directory = tempdir().expect("temporary directory");
    let store = VaultRegistryStore::new(directory.path().join("vaults.json"));
    let added = store
        .add(
            0,
            NewVaultDefinition {
                name: "Managed".to_string(),
                enabled: false,
                source: VaultSource::ManagedGit {
                    repository_url: "https://example.test/notes.git".to_string(),
                    branch: None,
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add managed Vault");
    let vault_id = added.definitions().next().expect("definition").vault_id();
    let managed_checkout = directory
        .path()
        .join("vaults")
        .join(vault_id.to_string())
        .join("repository");
    std::fs::create_dir_all(&managed_checkout).expect("create managed checkout location");

    let error = store
        .add(
            1,
            local_definition("Overlapping local", managed_checkout, true),
        )
        .expect_err("managed checkout overlap accepted");

    assert_eq!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::PathOverlap)
    );
}

#[cfg(unix)]
#[test]
fn absent_managed_checkout_resolves_a_symlinked_state_parent_for_overlap_checks() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("temporary directory");
    let real_state = directory.path().join("real-state");
    let reserved_checkouts = real_state.join("vaults");
    std::fs::create_dir_all(&reserved_checkouts).expect("create real state directory");
    let linked_state = directory.path().join("linked-state");
    symlink(&real_state, &linked_state).expect("symlink state directory");
    let store = VaultRegistryStore::new(linked_state.join("vaults.json"));
    store
        .add(
            0,
            NewVaultDefinition {
                name: "Managed".to_string(),
                enabled: false,
                source: VaultSource::ManagedGit {
                    repository_url: "https://example.test/notes.git".to_string(),
                    branch: None,
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add managed Vault before checkout acquisition");

    let error = store
        .add(
            1,
            local_definition("Overlapping local", reserved_checkouts, true),
        )
        .expect_err("symlink-aliased managed checkout parent accepted");

    assert_eq!(
        error,
        VaultRegistryError::InvalidDefinition(VaultDefinitionError::PathOverlap)
    );
}

fn local_definition(name: &str, path: PathBuf, enabled: bool) -> NewVaultDefinition {
    NewVaultDefinition {
        name: name.to_string(),
        enabled,
        source: VaultSource::Local { path },
        exclude_patterns: Vec::new(),
        https_credentials: None,
        archive_folder: None,
        commit_identity: None,
    }
}

fn managed_git_definition(name: &str) -> NewVaultDefinition {
    NewVaultDefinition {
        name: name.to_string(),
        enabled: true,
        source: VaultSource::ManagedGit {
            repository_url: "https://example.test/owner/notes.git".to_string(),
            branch: None,
            vault_subdirectory: None,
            mode: VaultGitMode::PullOnly,
            poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
        },
        exclude_patterns: Vec::new(),
        https_credentials: None,
        archive_folder: None,
        commit_identity: None,
    }
}

fn local_edit(name: &str, path: PathBuf, confirm_identity_change: bool) -> VaultDefinitionEdit {
    VaultDefinitionEdit {
        name: name.to_string(),
        source: VaultSource::Local { path },
        exclude_patterns: Vec::new(),
        https_credentials: HttpsCredentialUpdate::Keep,
        confirm_identity_change,
        archive_folder: None,
        commit_identity: None,
    }
}

#[test]
fn committed_registry_round_trips_ids_and_monotonic_revision() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("state").join("vaults.json");
    let store = VaultRegistryStore::new(&path);
    let id = VaultId::from_str("018f47a0-7768-4d0c-8da3-5aa28d1c31c7").expect("known Vault ID");

    let committed = store
        .commit(0, [(id, VaultRecord::empty(id))])
        .expect("commit registry");

    assert_eq!(committed.revision(), 1);
    assert_eq!(committed.vault_ids().collect::<Vec<_>>(), vec![id]);
    let encoded = std::fs::read_to_string(&path).expect("read registry");
    let encoded: serde_json::Value = serde_json::from_str(&encoded).expect("registry JSON");
    assert_eq!(encoded["schema_version"], 1);
    assert_eq!(encoded["revision"], 1);
    assert_eq!(
        encoded["vaults"]["018f47a0-7768-4d0c-8da3-5aa28d1c31c7"]["name"],
        "Test Vault 018f47a0-7768-4d0c-8da3-5aa28d1c31c7"
    );
    assert_eq!(
        encoded["vaults"]["018f47a0-7768-4d0c-8da3-5aa28d1c31c7"]["source"]["type"],
        "local"
    );

    let VaultRegistryState::Ready(restarted) = store.load().expect("reload registry") else {
        panic!("valid registry entered recovery");
    };
    assert_eq!(restarted, committed);
}

#[cfg(unix)]
#[test]
fn committed_registry_is_private_to_its_owner() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let store = VaultRegistryStore::new(&path);

    store.commit(0, []).expect("commit empty registry");

    let mode = std::fs::metadata(path)
        .expect("registry metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn stale_revision_is_rejected_without_changing_the_registry() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let store = VaultRegistryStore::new(&path);
    let initial = store.commit(0, []).expect("initial commit");
    let before = std::fs::read(&path).expect("registry before conflict");

    let error = store.commit(0, []).expect_err("stale commit accepted");

    assert_eq!(
        error,
        VaultRegistryError::RevisionConflict {
            expected: 0,
            actual: initial.revision(),
        }
    );
    assert_eq!(
        std::fs::read(path).expect("registry after conflict"),
        before
    );
}

#[test]
fn corrupt_registry_enters_recovery_and_is_never_overwritten() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let original = b"{ definitely not valid json";
    std::fs::write(&path, original).expect("write corrupt registry");
    let store = VaultRegistryStore::new(&path);

    let VaultRegistryState::Recovery(recovery) = store.load().expect("load registry") else {
        panic!("corrupt registry was treated as ready");
    };

    assert_eq!(recovery.kind(), VaultRegistryRecoveryKind::Corrupt);
    assert!(recovery.message().contains("corrupt"));
    assert!(recovery.message().contains("will not overwrite"));
    assert_eq!(
        store.commit(0, []).expect_err("corrupt file overwritten"),
        VaultRegistryError::RecoveryRequired
    );
    assert_eq!(std::fs::read(path).expect("retained registry"), original);
}

#[test]
fn future_schema_enters_recovery_with_upgrade_guidance() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let original = br#"{"schema_version":2,"revision":41,"vaults":{}}"#;
    std::fs::write(&path, original).expect("write future registry");
    let store = VaultRegistryStore::new(&path);

    let VaultRegistryState::Recovery(recovery) = store.load().expect("load registry") else {
        panic!("future registry was treated as ready");
    };

    assert_eq!(
        recovery.kind(),
        VaultRegistryRecoveryKind::FutureSchema {
            found: 2,
            supported: REGISTRY_SCHEMA_VERSION,
        }
    );
    assert!(recovery.message().contains("Upgrade Hatchdoor"));
    assert!(recovery.message().contains("will not overwrite"));
    assert_eq!(std::fs::read(path).expect("retained registry"), original);
}

#[test]
fn current_schema_with_unknown_record_fields_is_recoverable_corruption() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let original = br#"{
            "schema_version": 1,
            "revision": 3,
            "vaults": {
                "018f47a0-7768-4d0c-8da3-5aa28d1c31c7": {"name": "not-yet-supported"}
            }
        }"#;
    std::fs::write(&path, original).expect("write incompatible registry");
    let store = VaultRegistryStore::new(&path);

    let VaultRegistryState::Recovery(recovery) = store.load().expect("load registry") else {
        panic!("unsupported record shape was accepted");
    };

    assert_eq!(recovery.kind(), VaultRegistryRecoveryKind::Corrupt);
    assert_eq!(std::fs::read(path).expect("retained registry"), original);
}

#[test]
fn concurrent_commits_serialize_and_reject_one_stale_writer() {
    let directory = tempdir().expect("temporary directory");
    let store = Arc::new(VaultRegistryStore::new(
        directory.path().join("vaults.json"),
    ));
    store.commit(0, []).expect("initial commit");
    let barrier = Arc::new(Barrier::new(3));

    let writers = [
        "018f47a0-7768-4d0c-8da3-5aa28d1c31c7",
        "b676d7c4-ca1c-4c92-813f-b47b14a5192d",
    ]
    .map(|value| {
        let store = store.clone();
        let barrier = barrier.clone();
        let id = VaultId::from_str(value).expect("known Vault ID");
        std::thread::spawn(move || {
            barrier.wait();
            store.commit(1, [(id, VaultRecord::empty(id))])
        })
    });

    barrier.wait();
    let results = writers.map(|writer| writer.join().expect("writer thread"));
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| {
                matches!(
                    result,
                    Err(VaultRegistryError::RevisionConflict {
                        expected: 1,
                        actual: 2
                    })
                )
            })
            .count(),
        1
    );

    let VaultRegistryState::Ready(snapshot) = store.load().expect("load committed registry") else {
        panic!("committed registry entered recovery");
    };
    assert_eq!(snapshot.revision(), 2);
    assert_eq!(snapshot.vault_ids().count(), 1);
}

#[test]
fn independently_constructed_stores_serialize_writes_to_the_same_path() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let first = Arc::new(VaultRegistryStore::new(&path));
    let second = Arc::new(VaultRegistryStore::new(&path));
    first.commit(0, []).expect("initial commit");
    let records = (0..1024)
        .map(|_| {
            let id = VaultId::generate().expect("generate Vault ID");
            (id, VaultRecord::empty(id))
        })
        .collect::<Vec<_>>();
    let barrier = Arc::new(Barrier::new(3));

    let writers = [first, second].map(|store| {
        let barrier = barrier.clone();
        let records = records.clone();
        std::thread::spawn(move || {
            barrier.wait();
            store.commit(1, records)
        })
    });

    barrier.wait();
    let results = writers.map(|writer| writer.join().expect("writer thread"));
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| {
                matches!(
                    result,
                    Err(VaultRegistryError::RevisionConflict {
                        expected: 1,
                        actual: 2
                    })
                )
            })
            .count(),
        1
    );
}

#[test]
fn replacement_never_exposes_partial_json_to_readers() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("vaults.json");
    let store = Arc::new(VaultRegistryStore::new(&path));
    let vaults = (0..128)
        .map(|_| {
            let id = VaultId::generate().expect("generate Vault ID");
            (id, VaultRecord::empty(id))
        })
        .collect::<Vec<_>>();
    store.commit(0, vaults.clone()).expect("initial registry");
    let started = Arc::new(Barrier::new(2));
    let finished = Arc::new(AtomicBool::new(false));

    let writer = {
        let store = store.clone();
        let started = started.clone();
        let finished = finished.clone();
        std::thread::spawn(move || {
            started.wait();
            for expected in 1..=12 {
                let records = if expected % 2 == 0 {
                    vaults.clone()
                } else {
                    Vec::new()
                };
                store.commit(expected, records).expect("atomic commit");
            }
            finished.store(true, Ordering::Release);
        })
    };

    started.wait();
    let mut observations = 0;
    while !finished.load(Ordering::Acquire) || observations < 64 {
        let encoded = std::fs::read(&path).expect("read while replacing");
        let value: serde_json::Value =
            serde_json::from_slice(&encoded).expect("complete registry JSON");
        assert_eq!(value["schema_version"], REGISTRY_SCHEMA_VERSION);
        assert!(value["revision"].as_u64().is_some());
        assert!(matches!(
            value["vaults"].as_object().map(|vaults| vaults.len()),
            Some(0 | 128)
        ));
        observations += 1;
    }
    writer.join().expect("writer thread");

    let temporary_files = std::fs::read_dir(directory.path())
        .expect("registry directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(temporary_files, 0);
}
