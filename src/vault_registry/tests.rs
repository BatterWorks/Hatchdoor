use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

use tempfile::tempdir;

use super::{
    DEFAULT_VAULT_REGISTRY_PATH, HttpsCredentialUpdate, HttpsCredentials, NewVaultDefinition,
    REGISTRY_SCHEMA_VERSION, VaultDefinitionEdit, VaultDefinitionError, VaultGitMode, VaultId,
    VaultRecord, VaultRegistryError, VaultRegistryRecoveryKind, VaultRegistryState,
    VaultRegistryStore, VaultSource,
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: Some(credentials),
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: Some(credentials.clone()),
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: HttpsCredentialUpdate::Keep,
                confirm_identity_change: false,
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
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
                    },
                    exclude_patterns: Vec::new(),
                    https_credentials: None,
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
                },
                exclude_patterns: vec!["drafts/**".to_string()],
                https_credentials: HttpsCredentialUpdate::Replace(HttpsCredentials {
                    username: "new-user".to_string(),
                    token: "new-token".to_string(),
                }),
                confirm_identity_change: false,
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: Some(HttpsCredentials {
                    username: "old-user".to_string(),
                    token: "old-repository-token".to_string(),
                }),
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: HttpsCredentialUpdate::Keep,
                confirm_identity_change: true,
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
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
    }
}

fn local_edit(name: &str, path: PathBuf, confirm_identity_change: bool) -> VaultDefinitionEdit {
    VaultDefinitionEdit {
        name: name.to_string(),
        source: VaultSource::Local { path },
        exclude_patterns: Vec::new(),
        https_credentials: HttpsCredentialUpdate::Keep,
        confirm_identity_change,
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
