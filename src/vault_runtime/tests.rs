use super::*;
use crate::vault_executor::dispatch_vault_index_turn;
use tempfile::tempdir;

use crate::cache::SqliteCache;
use crate::cache::vault_snapshots::{VaultSnapshotFreshness, VaultSnapshotStatus};
use crate::embed::{Embedder, StubEmbedder};
use crate::vault_registry::{
    DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS, HttpsCredentialUpdate, NewVaultDefinition,
    VaultDefinitionEdit, VaultGitMode, VaultRegistrySnapshot, VaultRegistryStore,
    VaultSource as RegistryVaultSource,
};
use crate::vault_work::VaultWorkError;

/// Issue #132: a large affected-paths list must be capped, with `total`
/// carrying the true count — never an unbounded list, and never a
/// truncated-looking one with no way to tell how much was cut.
#[test]
fn runtime_error_detail_bounds_affected_paths_and_carries_the_true_total() {
    let paths = (0..(MAX_REPORTED_SYNC_ERROR_PATHS + 7))
        .map(|index| format!("note-{index}.md"))
        .collect::<Vec<_>>();
    let detail = VaultRuntimeErrorDetail::from(&VaultWorkErrorDetail::AffectedPaths(paths.clone()));
    match detail {
        VaultRuntimeErrorDetail::AffectedPaths {
            paths: reported,
            total,
        } => {
            assert_eq!(reported.len(), MAX_REPORTED_SYNC_ERROR_PATHS);
            assert_eq!(reported, &paths[..MAX_REPORTED_SYNC_ERROR_PATHS]);
            assert_eq!(total, paths.len());
        }
        other => panic!("expected AffectedPaths, got {other:?}"),
    }
}

/// A path count at or under the cap is carried through unchanged.
#[test]
fn runtime_error_detail_does_not_truncate_at_or_under_the_cap() {
    let paths = vec!["a.md".to_string(), "b.md".to_string()];
    let detail = VaultRuntimeErrorDetail::from(&VaultWorkErrorDetail::AffectedPaths(paths.clone()));
    assert_eq!(
        detail,
        VaultRuntimeErrorDetail::AffectedPaths {
            total: paths.len(),
            paths,
        }
    );
}

#[test]
fn runtime_error_detail_carries_the_local_commits_ahead_count_through_unbounded() {
    let detail = VaultRuntimeErrorDetail::from(&VaultWorkErrorDetail::LocalCommitsAhead(5));
    assert_eq!(
        detail,
        VaultRuntimeErrorDetail::LocalCommitsAhead { ahead: 5 }
    );
}

/// Issue #132's last acceptance criterion: "a dirty working copy and a
/// conflict both report their affected paths as data" — the actual wire
/// shape a caller receives (this is what `VaultSummary.git_error` and MCP
/// `list_vaults` both serialize verbatim, per `vault_summary`'s
/// `git_error: snapshot.git_error.clone()`). `detail` must be a tagged
/// object when present, and omitted — not serialized as `null` — for every
/// other code.
#[test]
fn runtime_error_detail_serializes_as_tagged_json_and_is_omitted_when_absent() {
    let with_paths = VaultRuntimeError {
        code: "managed_git_dirty_working_copy".to_string(),
        message: "x".to_string(),
        retryable: false,
        detail: Some(VaultRuntimeErrorDetail::AffectedPaths {
            paths: vec!["a.md".to_string()],
            total: 1,
        }),
    };
    let json = serde_json::to_value(&with_paths).expect("serialize");
    assert_eq!(
        json["detail"],
        serde_json::json!({"kind": "affected_paths", "paths": ["a.md"], "total": 1})
    );

    let with_count = VaultRuntimeError {
        code: "managed_git_pull_only_local_commits".to_string(),
        message: "x".to_string(),
        retryable: false,
        detail: Some(VaultRuntimeErrorDetail::LocalCommitsAhead { ahead: 4 }),
    };
    let json = serde_json::to_value(&with_count).expect("serialize");
    assert_eq!(
        json["detail"],
        serde_json::json!({"kind": "local_commits_ahead", "ahead": 4})
    );

    let without_detail = VaultRuntimeError {
        code: "managed_git_authentication_failed".to_string(),
        message: "x".to_string(),
        retryable: false,
        detail: None,
    };
    let json = serde_json::to_value(&without_detail).expect("serialize");
    assert!(
        json.get("detail").is_none(),
        "detail must be omitted, not serialized as null, for a code that carries none"
    );
}

fn local_source() -> VaultSource {
    VaultSource::Local {
        vault_path: PathBuf::from("/data/vault"),
    }
}

#[test]
fn startup_source_never_claims_a_git_capability() {
    // Git is per-Vault and derived from the registry definition; the process
    // source pulls and pushes nothing regardless of phase.
    let capabilities = VaultRuntime::ready(local_source()).snapshot().capabilities;
    assert!(capabilities.browse);
    assert!(capabilities.search);
    assert!(capabilities.mutate);
    assert!(!capabilities.pull);
    assert!(!capabilities.push);
}

#[test]
fn unavailable_state_has_no_ready_vault_capabilities() {
    let runtime = VaultRuntime::new(local_source());
    runtime.set_unavailable("not_acquired", "Vault has not been acquired");
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.phase, VaultPhase::Unavailable);
    assert!(!snapshot.capabilities.browse);
    assert!(!snapshot.capabilities.search);
    assert!(!snapshot.capabilities.mutate);
    assert!(!snapshot.capabilities.retry);
}

#[test]
fn local_history_ready_state_never_exposes_remote_capabilities() {
    let directory = tempdir().expect("temporary state directory");
    let repository_path = directory.path().join("repository");
    git2::Repository::init(&repository_path).expect("initialize repository");
    let vault_path = repository_path.join("notes");
    std::fs::create_dir(&vault_path).expect("create Vault subdirectory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let snapshot = registry
        .add(
            0,
            NewVaultDefinition {
                name: "Local history".to_string(),
                enabled: true,
                source: RegistryVaultSource::ExistingGit {
                    repository_path,
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
        .expect("add local-history Vault");
    let vault_id = vault_id_named(&snapshot, "Local history");
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &snapshot);
    let runtime = collection.runtime(vault_id).expect("active runtime");

    runtime
        .set_git_status(VaultGitStatus::Ready, None)
        .expect("publish ready Git status");

    let capabilities = runtime.snapshot().capabilities;
    assert!(capabilities.mutate);
    assert!(!capabilities.pull);
    assert!(!capabilities.push);
}

fn add_local_vault(
    registry: &VaultRegistryStore,
    snapshot: &VaultRegistrySnapshot,
    name: &str,
    path: PathBuf,
) -> VaultRegistrySnapshot {
    registry
        .add(
            snapshot.revision(),
            NewVaultDefinition {
                name: name.to_string(),
                enabled: true,
                source: RegistryVaultSource::Local { path },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add local Vault")
}

fn vault_id_named(snapshot: &VaultRegistrySnapshot, name: &str) -> VaultId {
    snapshot
        .definitions()
        .find(|definition| definition.name() == name)
        .expect("named Vault definition")
        .vault_id()
}

#[test]
fn activates_zero_one_and_many_enabled_vaults_from_registry_snapshots() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => {
            panic!("new registry entered recovery")
        }
    };
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &empty);
    assert!(collection.snapshot().vaults.is_empty());

    let first_path = directory.path().join("first");
    std::fs::create_dir_all(&first_path).expect("first Vault directory");
    let one = add_local_vault(&registry, &empty, "First", first_path);
    collection.reconcile(&registry, &one);
    assert_eq!(collection.snapshot().vaults.len(), 1);
    assert_eq!(collection.active_vault_ids().len(), 1);

    let second_path = directory.path().join("second");
    std::fs::create_dir_all(&second_path).expect("second Vault directory");
    let many = add_local_vault(&registry, &one, "Second", second_path);
    collection.reconcile(&registry, &many);
    assert_eq!(collection.snapshot().vaults.len(), 2);
    assert_eq!(collection.active_vault_ids().len(), 2);
    assert!(
        collection
            .snapshot()
            .vaults
            .values()
            .all(|vault| vault.capabilities.browse)
    );
}

#[test]
fn activation_failure_is_isolated_from_healthy_local_markdown() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let healthy_path = directory.path().join("healthy");
    std::fs::create_dir_all(&healthy_path).expect("healthy Vault directory");
    let one = add_local_vault(&registry, &empty, "Healthy", healthy_path);
    let two = registry
        .add(
            one.revision(),
            NewVaultDefinition {
                name: "Unavailable managed".to_string(),
                enabled: true,
                source: RegistryVaultSource::ManagedGit {
                    repository_url: "https://example.test/vault.git".to_string(),
                    branch: Some("main".to_string()),
                    vault_subdirectory: None,
                    mode: VaultGitMode::TwoWay,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add managed Vault before acquisition");

    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &two);
    let snapshot = collection.snapshot();
    let healthy = &snapshot.vaults[&vault_id_named(&two, "Healthy")];
    let unavailable = &snapshot.vaults[&vault_id_named(&two, "Unavailable managed")];
    assert_eq!(healthy.activation, VaultActivationStatus::Active);
    assert!(healthy.capabilities.browse);
    assert_eq!(unavailable.activation, VaultActivationStatus::Unavailable);
    assert!(!unavailable.capabilities.browse);
    assert_eq!(
        unavailable
            .activation_error
            .as_ref()
            .map(|error| error.code.as_str()),
        Some("vault_path_unavailable")
    );
}

#[test]
fn read_only_and_stale_statuses_keep_usable_local_markdown_honest() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("read-only");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    let original_mode = std::fs::metadata(&vault_path)
        .expect("Vault metadata")
        .permissions()
        .mode();
    std::fs::set_permissions(&vault_path, std::fs::Permissions::from_mode(0o555))
        .expect("make Vault read-only");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let one = add_local_vault(&registry, &empty, "Read only", vault_path.clone());
    let vault_id = vault_id_named(&one, "Read only");
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &one);
    let runtime = collection.runtime(vault_id).expect("enabled runtime");

    let read_only = runtime.snapshot();
    assert_eq!(read_only.local_content, LocalContentStatus::ReadOnly);
    assert!(read_only.capabilities.browse);
    assert!(!read_only.capabilities.mutate);
    assert!(!read_only.capabilities.search);

    runtime
        .set_search_status(VaultSearchStatus::Stale, None)
        .expect("publish stale search status");
    let stale = runtime.snapshot();
    assert_eq!(stale.search, VaultSearchStatus::Stale);
    assert!(stale.capabilities.browse);
    assert!(stale.capabilities.search);
    assert!(!stale.capabilities.mutate);

    runtime
        .set_git_status(
            VaultGitStatus::Unavailable,
            Some(VaultRuntimeError {
                code: "git_temporarily_unavailable".to_string(),
                message: "Git is temporarily unavailable".to_string(),
                retryable: true,
                detail: None,
            }),
        )
        .expect("publish unavailable Git status");
    let git_degraded = runtime.snapshot();
    assert!(git_degraded.capabilities.browse);
    assert!(git_degraded.capabilities.search);
    assert!(!git_degraded.capabilities.mutate);
    assert!(git_degraded.capabilities.retry);
    assert_eq!(
        git_degraded
            .git_error
            .as_ref()
            .map(|error| error.code.as_str()),
        Some("git_temporarily_unavailable")
    );
    assert!(git_degraded.search_error.is_none());

    std::fs::set_permissions(&vault_path, std::fs::Permissions::from_mode(original_mode))
        .expect("restore Vault permissions");
}

#[test]
fn set_local_content_status_makes_a_managed_vault_browsable_after_first_acquisition() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let committed = registry
        .add(
            empty.revision(),
            NewVaultDefinition {
                name: "Unacquired managed".to_string(),
                enabled: true,
                source: RegistryVaultSource::ManagedGit {
                    repository_url: "https://example.test/vault.git".to_string(),
                    branch: Some("main".to_string()),
                    vault_subdirectory: None,
                    mode: VaultGitMode::TwoWay,
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add managed Vault before acquisition");
    let vault_id = vault_id_named(&committed, "Unacquired managed");
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &committed);
    let runtime = collection.runtime(vault_id).expect("active runtime");

    let before_acquisition = runtime.snapshot();
    assert_eq!(
        before_acquisition.activation,
        VaultActivationStatus::Unavailable
    );
    assert_eq!(
        before_acquisition.local_content,
        LocalContentStatus::Unavailable
    );
    assert!(!before_acquisition.capabilities.browse);

    // The Git lifecycle packet materializes the checkout at the path the
    // registry already resolved, then publishes it — no `reconcile()`
    // needed, since the definition itself never changed.
    let vault_path = registry.vault_path(runtime.definition());
    std::fs::create_dir_all(&vault_path).expect("acquired checkout Vault root");
    runtime
        .set_local_content_status(LocalContentStatus::ReadWrite, None)
        .expect("publish acquired local content");

    let acquired = runtime.snapshot();
    assert_eq!(acquired.activation, VaultActivationStatus::Active);
    assert_eq!(acquired.local_content, LocalContentStatus::ReadWrite);
    assert!(acquired.activation_error.is_none());
    assert!(acquired.capabilities.browse);
    // Git status is untouched by this seam — it remains whatever the Git
    // lifecycle packet last published (still `Pending` here).
    assert_eq!(acquired.git, VaultGitStatus::Pending);

    // A later lost checkout degrades local content independently again.
    runtime
        .set_local_content_status(
            LocalContentStatus::Unavailable,
            Some(VaultRuntimeError {
                code: "vault_path_unavailable".to_string(),
                message: "checkout directory disappeared".to_string(),
                retryable: true,
                detail: None,
            }),
        )
        .expect("publish lost local content");
    let lost = runtime.snapshot();
    assert_eq!(lost.activation, VaultActivationStatus::Unavailable);
    assert!(!lost.capabilities.browse);
    assert_eq!(
        lost.activation_error
            .as_ref()
            .map(|error| error.code.as_str()),
        Some("vault_path_unavailable")
    );
}

/// Closes a Spec-review finding on issue #97's reopening findings 1/2: an
/// in-place edit to a non-identity field (anything that does not require
/// disabling the Vault first — interval, name, exclude patterns, mode,
/// credentials) on an already-active managed-Git Vault constructs a fresh
/// `VaultControlBlock` (any definition change breaks `reconcile()`'s
/// ptr_eq retention), but must carry the *retiring* control block's actual
/// current Git status through to the replacement rather than resetting it
/// to `Pending`. Forcing `Pending` would make the active loop request an
/// immediate real Git turn regardless of an armed backoff or other real
/// status — silently bypassing finding 1's whole point. `Pending` remains
/// correct for a genuinely new Vault or a disabled-to-enabled transition;
/// see `disabling_and_reenabling_a_managed_git_vault_forces_a_fresh_pending_sync`
/// below for that complementary case.
#[test]
fn editing_a_non_identity_field_preserves_the_vaults_actual_prior_git_status() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let committed = registry
        .add(
            empty.revision(),
            NewVaultDefinition {
                name: "Remote notes".to_string(),
                enabled: true,
                source: RegistryVaultSource::ManagedGit {
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
            },
        )
        .expect("add managed Vault");
    let vault_id = vault_id_named(&committed, "Remote notes");
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &committed);
    let runtime = collection.runtime(vault_id).expect("active runtime");
    assert_eq!(runtime.snapshot().git, VaultGitStatus::Pending);

    // A real turn moves the Vault to `Unavailable` mid-backoff from a
    // transient failure — exactly the status a benign edit below must not
    // silently discard.
    let transient_error = VaultRuntimeError {
        code: "managed_git_remote_unreachable".to_string(),
        message: "temporary DNS failure".to_string(),
        retryable: true,
        detail: None,
    };
    runtime
        .set_git_status(VaultGitStatus::Unavailable, Some(transient_error.clone()))
        .expect("publish a real transient Git failure");

    // Edit only the poll interval: a non-identity field, so the Vault stays
    // enabled and active with the same remote identity throughout.
    let edited = registry
        .edit(
            committed.revision(),
            vault_id,
            VaultDefinitionEdit {
                name: "Remote notes".to_string(),
                source: RegistryVaultSource::ManagedGit {
                    repository_url: "https://example.test/owner/notes.git".to_string(),
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
        .expect("edit only the poll interval");
    collection.reconcile(&registry, &edited);

    let after_edit = collection
        .runtime(vault_id)
        .expect("Vault remains active after a non-identity edit")
        .snapshot();
    assert_eq!(
        after_edit.git,
        VaultGitStatus::Unavailable,
        "a benign edit must not force the Git status back to Pending, \
         which would trigger an unwanted immediate resync"
    );
    assert_eq!(
        after_edit
            .git_error
            .as_ref()
            .map(|error| error.code.as_str()),
        Some("managed_git_remote_unreachable"),
        "the actual prior error must be carried over too, not just the status enum"
    );
}

/// Complements
/// `editing_a_non_identity_field_preserves_the_vaults_actual_prior_git_status`:
/// a Vault that goes disabled then re-enabled again is not an in-place edit
/// of a still-active Vault — it genuinely leaves and rejoins the active
/// collection — so it must still get a fresh `Pending` status (an
/// immediate first sync) exactly as a brand-new Vault does, regardless of
/// whatever Git status it had before being disabled.
#[test]
fn disabling_and_reenabling_a_managed_git_vault_forces_a_fresh_pending_sync() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let committed = registry
        .add(
            empty.revision(),
            NewVaultDefinition {
                name: "Remote notes".to_string(),
                enabled: true,
                source: RegistryVaultSource::ManagedGit {
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
            },
        )
        .expect("add managed Vault");
    let vault_id = vault_id_named(&committed, "Remote notes");
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &committed);
    let runtime = collection.runtime(vault_id).expect("active runtime");
    runtime
        .set_git_status(
            VaultGitStatus::Unavailable,
            Some(VaultRuntimeError {
                code: "managed_git_remote_unreachable".to_string(),
                message: "x".to_string(),
                retryable: true,
                detail: None,
            }),
        )
        .expect("publish a real Git failure before disabling");

    let disabled = registry
        .disable(committed.revision(), vault_id)
        .expect("disable");
    collection.reconcile(&registry, &disabled);
    assert!(collection.runtime(vault_id).is_none());

    let reenabled = registry
        .enable(disabled.revision(), vault_id)
        .expect("enable");
    collection.reconcile(&registry, &reenabled);
    let runtime = collection.runtime(vault_id).expect("reenabled runtime");
    assert_eq!(
        runtime.snapshot().git,
        VaultGitStatus::Pending,
        "a Vault transitioning from disabled to enabled must get a fresh Pending \
         status and an immediate first sync, not whatever Git status it had before disabling"
    );
}

#[tokio::test]
async fn status_changes_advance_and_publish_collection_revisions() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let one = add_local_vault(&registry, &empty, "Vault", vault_path);
    let vault_id = vault_id_named(&one, "Vault");
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &one);
    let mut revisions = collection.subscribe_revisions();
    let before = collection.snapshot().collection_revision;

    collection
        .runtime(vault_id)
        .expect("enabled runtime")
        .set_search_status(VaultSearchStatus::Ready, None)
        .expect("publish ready search status");

    revisions.changed().await.expect("revision event");
    let after = collection.snapshot().collection_revision;
    assert_eq!(after, before + 1);
    let event = revisions.borrow_and_update().clone();
    assert_eq!(event.collection_revision, after);
    assert_eq!(event.vault_ids, vec![vault_id]);
    assert_eq!(event.category, VaultChangeCategory::Status);
    assert_eq!(collection.snapshot().registry_revision, one.revision());
}

#[tokio::test]
async fn reconcile_event_reports_only_the_vault_ids_that_actually_changed() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let first_path = directory.path().join("first");
    std::fs::create_dir_all(&first_path).expect("first Vault directory");
    let one = add_local_vault(&registry, &empty, "First", first_path);
    let first_id = vault_id_named(&one, "First");
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &one);
    let mut revisions = collection.subscribe_revisions();
    revisions.borrow_and_update();

    let second_path = directory.path().join("second");
    std::fs::create_dir_all(&second_path).expect("second Vault directory");
    let two = add_local_vault(&registry, &one, "Second", second_path);
    let second_id = vault_id_named(&two, "Second");
    collection.reconcile(&registry, &two);

    revisions.changed().await.expect("definition event");
    let event = revisions.borrow_and_update().clone();
    assert_eq!(event.category, VaultChangeCategory::Definition);
    assert_eq!(event.vault_ids, vec![second_id]);
    assert!(!event.vault_ids.contains(&first_id));
}

#[tokio::test]
async fn disabled_runtime_rejects_operations_through_preexisting_handles() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let one = add_local_vault(&registry, &empty, "Vault", vault_path);
    let vault_id = vault_id_named(&one, "Vault");
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &one);
    let runtime = collection.runtime(vault_id).expect("enabled runtime");
    let mut cancellation = runtime.subscribe_cancellation();

    let disabled = registry
        .disable(one.revision(), vault_id)
        .expect("disable Vault");
    collection.reconcile(&registry, &disabled);

    cancellation.changed().await.expect("cancellation event");
    assert!(*cancellation.borrow_and_update());
    assert!(*runtime.subscribe_cancellation().borrow());
    assert!(!runtime.is_accepting_operations());
    assert_eq!(
        runtime
            .acquire_mutation()
            .await
            .expect_err("disabled runtime must reject mutation")
            .code,
        "vault_runtime_not_active"
    );
    assert_eq!(
        runtime
            .set_search_status(VaultSearchStatus::Ready, None)
            .expect_err("disabled runtime must reject status changes")
            .code,
        "vault_runtime_not_active"
    );
}

#[test]
fn unreadable_directory_is_not_reported_as_browseable() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("unreadable");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let one = add_local_vault(&registry, &empty, "Unreadable", vault_path.clone());
    std::fs::set_permissions(&vault_path, std::fs::Permissions::from_mode(0o077))
        .expect("deny the owning process access after registration");
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &one);
    let status = &collection.snapshot().vaults[&vault_id_named(&one, "Unreadable")];

    assert_eq!(status.local_content, LocalContentStatus::Unavailable);
    assert!(!status.capabilities.browse);
    std::fs::set_permissions(&vault_path, std::fs::Permissions::from_mode(0o700))
        .expect("restore Vault permissions");
}

#[tokio::test]
async fn disable_enable_and_disconnect_only_replace_the_target_runtime() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    std::fs::create_dir_all(&first_path).expect("first Vault directory");
    std::fs::create_dir_all(&second_path).expect("second Vault directory");
    let one = add_local_vault(&registry, &empty, "First", first_path);
    let two = add_local_vault(&registry, &one, "Second", second_path);
    let first_id = vault_id_named(&two, "First");
    let second_id = vault_id_named(&two, "Second");
    let collection = VaultCollectionRuntime::with_watching(directory.path().join("cache.sqlite3"));
    collection.reconcile(&registry, &two);
    let first_runtime = collection.runtime(first_id).expect("first runtime");
    let first_lock = first_runtime.mutation_lock.clone();
    let second_lock = collection
        .runtime(second_id)
        .expect("second runtime")
        .mutation_lock
        .clone();
    assert!(!Arc::ptr_eq(&first_lock, &second_lock));
    assert_eq!(
        first_runtime.snapshot().watcher,
        VaultWatcherStatus::Running
    );

    let disabled = registry
        .disable(two.revision(), first_id)
        .expect("disable first Vault");
    collection.reconcile(&registry, &disabled);
    assert!(collection.runtime(first_id).is_none());
    assert!(first_runtime.watcher_cancelled());
    assert!(Arc::ptr_eq(
        &second_lock,
        &collection
            .runtime(second_id)
            .expect("second runtime retained")
            .mutation_lock
    ));
    let disabled_status = &collection.snapshot().vaults[&first_id];
    assert_eq!(disabled_status.activation, VaultActivationStatus::Disabled);
    assert_eq!(disabled_status.watcher, VaultWatcherStatus::Disabled);
    assert_eq!(disabled_status.capabilities, VaultCapabilities::default());

    let enabled = registry
        .enable(disabled.revision(), first_id)
        .expect("enable first Vault");
    collection.reconcile(&registry, &enabled);
    assert!(collection.runtime(first_id).is_some());
    assert!(Arc::ptr_eq(
        &second_lock,
        &collection
            .runtime(second_id)
            .expect("second runtime still retained")
            .mutation_lock
    ));

    let disconnected = registry
        .disconnect(enabled.revision(), first_id)
        .expect("disconnect first Vault");
    collection.reconcile(&registry, &disconnected);
    assert!(!collection.snapshot().vaults.contains_key(&first_id));
    assert!(Arc::ptr_eq(
        &second_lock,
        &collection
            .runtime(second_id)
            .expect("second runtime survives disconnect")
            .mutation_lock
    ));
}

#[tokio::test]
async fn lifecycle_retirement_updates_only_the_target_published_snapshot() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    for path in [&first_path, &second_path] {
        std::fs::create_dir_all(path).expect("Vault directory");
        std::fs::write(path.join("Home.md"), "# Home\n\nsearchable").expect("Vault note");
    }
    let one = add_local_vault(&registry, &empty, "First", first_path.clone());
    let two = add_local_vault(&registry, &one, "Second", second_path.clone());
    let first_id = vault_id_named(&two, "First");
    let second_id = vault_id_named(&two, "Second");
    let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));
    let embedder = StubEmbedder::new(384);
    for (vault_id, path) in [(first_id, &first_path), (second_id, &second_path)] {
        cache
            .replace_vault_snapshot(
                vault_id,
                &crate::vault::VaultIndex::build(path).expect("build Vault index"),
                &embedder,
            )
            .expect("publish Vault snapshot");
    }
    let collection = VaultCollectionRuntime::with_watching_and_cache(
        directory.path().join("cache.sqlite3"),
        cache.clone(),
    );
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &two, &coordinator, &managed_git)
        .await;

    let disabled = registry
        .disable(two.revision(), first_id)
        .expect("disable first Vault");
    collection
        .reconcile_and_reconstruct(&registry, &disabled, &coordinator, &managed_git)
        .await;
    assert_eq!(
        cache.snapshot_status(first_id).expect("first status"),
        Some(VaultSnapshotStatus {
            participating: false,
            freshness: VaultSnapshotFreshness::Fresh,
            searchable: true,
        })
    );
    assert!(
        cache
            .snapshot_status(second_id)
            .expect("second status")
            .expect("second snapshot")
            .participating
    );

    let enabled = registry
        .enable(disabled.revision(), first_id)
        .expect("enable first Vault");
    collection
        .reconcile_and_reconstruct(&registry, &enabled, &coordinator, &managed_git)
        .await;
    assert!(
        !cache
            .snapshot_status(first_id)
            .expect("first status after re-enable")
            .expect("retained first snapshot")
            .participating,
        "re-enabling must wait for successful Index publication"
    );

    let disconnected = registry
        .disconnect(enabled.revision(), first_id)
        .expect("disconnect first Vault");
    collection
        .reconcile_and_reconstruct(&registry, &disconnected, &coordinator, &managed_git)
        .await;
    assert_eq!(cache.snapshot_status(first_id).expect("first status"), None);
    assert_eq!(
        cache.snapshot_note_count(second_id).expect("second notes"),
        1
    );
}

#[test]
fn an_older_registry_snapshot_cannot_replace_a_newer_live_collection() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    std::fs::create_dir_all(&first_path).expect("first Vault directory");
    std::fs::create_dir_all(&second_path).expect("second Vault directory");
    let one = add_local_vault(&registry, &empty, "First", first_path);
    let two = add_local_vault(&registry, &one, "Second", second_path);
    let second_id = vault_id_named(&two, "Second");
    let collection = VaultCollectionRuntime::new();

    collection.reconcile(&registry, &one);
    collection.reconcile(&registry, &two);
    collection.reconcile(&registry, &one);

    let live = collection.snapshot();
    assert_eq!(live.registry_revision, two.revision());
    assert!(live.vaults.contains_key(&second_id));
}

#[tokio::test]
async fn disabling_a_vault_waits_for_an_active_foreground_mutation_safe_boundary() {
    use crate::vault_work::VaultWorkCoordinator;

    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let enabled = add_local_vault(&registry, &empty, "Vault", vault_path);
    let vault_id = vault_id_named(&enabled, "Vault");
    let collection = VaultCollectionRuntime::new();
    let (coordinator, _) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &enabled, &coordinator, &managed_git)
        .await;
    let runtime = collection.runtime(vault_id).expect("enabled runtime");
    let mutation = runtime
        .acquire_mutation()
        .await
        .expect("foreground mutation acquires its Vault lock");
    let disabled = registry
        .disable(enabled.revision(), vault_id)
        .expect("disable Vault");

    let reconciliation =
        collection.reconcile_and_reconstruct(&registry, &disabled, &coordinator, &managed_git);
    tokio::pin!(reconciliation);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut reconciliation)
            .await
            .is_err(),
        "disable waits for the active foreground mutation"
    );
    assert!(!runtime.is_accepting_operations());

    drop(mutation);
    reconciliation.await;
    assert!(collection.runtime(vault_id).is_none());
}

#[tokio::test]
async fn shutdown_waits_for_an_active_foreground_mutation_safe_boundary() {
    use crate::vault_work::VaultWorkCoordinator;

    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let enabled = add_local_vault(&registry, &empty, "Vault", vault_path);
    let vault_id = vault_id_named(&enabled, "Vault");
    let collection = VaultCollectionRuntime::new();
    let (coordinator, _) = VaultWorkCoordinator::new();
    collection.reconcile(&registry, &enabled);
    let runtime = collection.runtime(vault_id).expect("enabled runtime");
    let mutation = runtime
        .acquire_mutation()
        .await
        .expect("foreground mutation acquires its Vault lock");

    let shutdown = collection.shutdown(&coordinator);
    tokio::pin!(shutdown);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut shutdown)
            .await
            .is_err(),
        "shutdown waits for the active foreground mutation"
    );
    assert!(!runtime.is_accepting_operations());

    drop(mutation);
    shutdown.await;
}

#[tokio::test]
async fn an_older_reconciliation_cannot_readmit_work_after_a_newer_snapshot_applies() {
    use crate::vault_work::{ScheduleResult, VaultWorkCoordinator, VaultWorkKind};

    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let enabled = add_local_vault(&registry, &empty, "Vault", vault_path.clone());
    let vault_id = vault_id_named(&enabled, "Vault");
    let collection = VaultCollectionRuntime::new();
    let (coordinator, _) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &enabled, &coordinator, &managed_git)
        .await;
    let original = collection.runtime(vault_id).expect("enabled runtime");
    let mutation = original
        .acquire_mutation()
        .await
        .expect("foreground mutation acquires its Vault lock");
    let replacement = registry
        .edit(
            enabled.revision(),
            vault_id,
            VaultDefinitionEdit {
                name: "Vault".to_string(),
                source: RegistryVaultSource::Local { path: vault_path },
                exclude_patterns: vec!["ignored/**".to_string()],
                https_credentials: HttpsCredentialUpdate::Keep,
                confirm_identity_change: false,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("replace enabled Vault definition");
    let older =
        collection.reconcile_and_reconstruct(&registry, &replacement, &coordinator, &managed_git);
    tokio::pin!(older);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut older)
            .await
            .is_err(),
        "replacement waits at the old mutation boundary"
    );
    let disabled = registry
        .disable(replacement.revision(), vault_id)
        .expect("disable replacement Vault");
    collection
        .reconcile_and_reconstruct(&registry, &disabled, &coordinator, &managed_git)
        .await;

    drop(mutation);
    older.await;

    assert!(collection.runtime(vault_id).is_none());
    assert_eq!(
        coordinator.request(vault_id, VaultWorkKind::Index),
        ScheduleResult::Rejected,
        "the resumed older reconciliation cannot re-admit retired work"
    );
}

#[tokio::test]
async fn restart_reconstructs_index_work_for_each_enabled_vault_from_the_collection() {
    use crate::vault_work::{VaultWorkCoordinator, VaultWorkError, VaultWorkKind};

    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    std::fs::create_dir_all(&first_path).expect("first Vault directory");
    std::fs::create_dir_all(&second_path).expect("second Vault directory");
    let one = add_local_vault(&registry, &empty, "First", first_path);
    let two = add_local_vault(&registry, &one, "Second", second_path);
    let mut expected = [
        vault_id_named(&two, "First"),
        vault_id_named(&two, "Second"),
    ];
    expected.sort();
    let collection = VaultCollectionRuntime::new();
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());

    collection
        .reconcile_and_reconstruct(&registry, &two, &coordinator, &managed_git)
        .await;

    let mut reconstructed = Vec::new();
    for _ in expected {
        let turn = worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .expect("reconstructed turn");
        assert_eq!(turn.request.kind(), VaultWorkKind::Index);
        reconstructed.push(turn.request.vault_id());
    }
    assert_eq!(reconstructed, expected);
}

#[tokio::test]
async fn restart_reports_retained_cache_freshness_while_reconstructing_index_work() {
    use crate::vault_work::{VaultWorkCoordinator, VaultWorkError, VaultWorkKind};

    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let fresh_path = directory.path().join("fresh");
    let stale_path = directory.path().join("stale");
    let absent_path = directory.path().join("absent");
    for path in [&fresh_path, &stale_path, &absent_path] {
        std::fs::create_dir_all(path).expect("Vault directory");
        std::fs::write(path.join("Home.md"), "# Home\n\ncontent").expect("Vault note");
    }
    let one = add_local_vault(&registry, &empty, "Fresh", fresh_path.clone());
    let two = add_local_vault(&registry, &one, "Stale", stale_path.clone());
    let three = add_local_vault(&registry, &two, "Absent", absent_path);
    let fresh_id = vault_id_named(&three, "Fresh");
    let stale_id = vault_id_named(&three, "Stale");
    let absent_id = vault_id_named(&three, "Absent");

    let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));
    let embedder = StubEmbedder::new(384);
    cache
        .replace_vault_snapshot(
            fresh_id,
            &crate::vault::VaultIndex::build(&fresh_path).expect("build fresh index"),
            &embedder,
        )
        .expect("publish fresh snapshot");
    cache
        .replace_vault_snapshot(
            stale_id,
            &crate::vault::VaultIndex::build(&stale_path).expect("build stale index"),
            &embedder,
        )
        .expect("publish stale snapshot");
    cache
        .mark_vault_snapshot_stale(stale_id)
        .expect("mark retained snapshot stale");

    let collection = VaultCollectionRuntime::with_watching_and_cache(
        directory.path().join("cache.sqlite3"),
        cache,
    );
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &three, &coordinator, &managed_git)
        .await;

    let runtime = collection.snapshot();
    assert_eq!(runtime.vaults[&fresh_id].search, VaultSearchStatus::Ready);
    assert!(runtime.vaults[&fresh_id].capabilities.search);
    assert_eq!(runtime.vaults[&stale_id].search, VaultSearchStatus::Stale);
    assert!(runtime.vaults[&stale_id].capabilities.search);
    assert_eq!(
        runtime.vaults[&absent_id].search,
        VaultSearchStatus::Unavailable
    );
    assert!(!runtime.vaults[&absent_id].capabilities.search);

    let mut reconstructed = Vec::new();
    for _ in [fresh_id, stale_id, absent_id] {
        let turn = worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .expect("reconstructed Index turn");
        assert_eq!(turn.request.kind(), VaultWorkKind::Index);
        reconstructed.push(turn.request.vault_id());
    }
    reconstructed.sort();
    let mut expected = vec![fresh_id, stale_id, absent_id];
    expected.sort();
    assert_eq!(reconstructed, expected);
}

/// A restart landing between a Vault's structure pass and its embedding pass
/// finds a participating, fresh, vectorless generation. Reading freshness alone
/// would reconstruct it as `Ready`, advertising a search capability that would
/// answer every query with nothing.
#[tokio::test]
async fn restart_reconstructs_a_structure_only_snapshot_as_browsable_not_ready() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let vault_path = directory.path().join("browsable");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    std::fs::write(vault_path.join("Home.md"), "# Home\n\ncontent").expect("Vault note");
    let added = add_local_vault(&registry, &empty, "Browsable", vault_path.clone());
    let browsable_id = vault_id_named(&added, "Browsable");

    let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));
    let embedder = StubEmbedder::new(384);
    let published = cache
        .publish_vault_structure_snapshot(
            browsable_id,
            &crate::vault::VaultIndex::build(&vault_path).expect("build index"),
            &embedder,
            true,
        )
        .expect("publish structure-only snapshot");
    assert!(published);

    let collection = VaultCollectionRuntime::with_watching_and_cache(
        directory.path().join("cache.sqlite3"),
        cache,
    );
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &added, &coordinator, &managed_git)
        .await;

    let runtime = collection.snapshot();
    assert_eq!(
        runtime.vaults[&browsable_id].search,
        VaultSearchStatus::Browsable
    );
    assert!(
        !runtime.vaults[&browsable_id].capabilities.search,
        "a Vault with no vectors must not advertise search"
    );
    assert!(
        runtime.vaults[&browsable_id].capabilities.browse,
        "its Notes are published and readable"
    );
}

/// A structure pass that lands before its embedding pass fails leaves a
/// participating generation with no vectors. Reporting that `Stale` would
/// grant it the `search` capability, and semantic search would then answer
/// every query with nothing while claiming to be a working stale snapshot.
#[tokio::test]
async fn a_vectorless_generation_never_advertises_search_even_when_stale() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let vault_path = directory.path().join("browsable");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    std::fs::write(vault_path.join("Home.md"), "# Home\n\ncontent").expect("Vault note");
    let added = add_local_vault(&registry, &empty, "Browsable", vault_path.clone());
    let browsable_id = vault_id_named(&added, "Browsable");

    let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));
    let embedder = StubEmbedder::new(384);
    cache
        .publish_vault_structure_snapshot(
            browsable_id,
            &crate::vault::VaultIndex::build(&vault_path).expect("build index"),
            &embedder,
            true,
        )
        .expect("publish structure-only snapshot");
    // What a failed embedding pass leaves behind.
    cache
        .mark_vault_snapshot_stale(browsable_id)
        .expect("mark the vectorless generation stale");

    let collection = VaultCollectionRuntime::with_watching_and_cache(
        directory.path().join("cache.sqlite3"),
        cache,
    );
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &added, &coordinator, &managed_git)
        .await;

    let runtime = collection.snapshot();
    assert_eq!(
        runtime.vaults[&browsable_id].search,
        VaultSearchStatus::Browsable,
        "vectorless outranks stale: a generation with no vectors is not a searchable one"
    );
    assert!(
        !runtime.vaults[&browsable_id].capabilities.search,
        "a Vault with no vectors must never advertise search"
    );
    assert!(runtime.vaults[&browsable_id].capabilities.browse);
}

#[tokio::test]
async fn disabling_a_vault_waits_for_its_active_work_safe_boundary() {
    use std::sync::Arc;

    use crate::vault_work::{ScheduleResult, VaultWorkCoordinator, VaultWorkError};

    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let enabled = add_local_vault(&registry, &empty, "Vault", vault_path);
    let vault_id = vault_id_named(&enabled, "Vault");
    let collection = VaultCollectionRuntime::new();
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &enabled, &coordinator, &managed_git)
        .await;

    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let running = tokio::spawn({
        let started = started.clone();
        let release = release.clone();
        async move {
            worker
                .run_next(move |request| {
                    let started = started.clone();
                    let release = release.clone();
                    async move {
                        assert_eq!(request.vault_id(), vault_id);
                        started.notify_one();
                        release.notified().await;
                        Ok::<(), VaultWorkError>(())
                    }
                })
                .await
        }
    });
    started.notified().await;

    let disabled = registry
        .disable(enabled.revision(), vault_id)
        .expect("disable Vault");
    let reconciliation =
        collection.reconcile_and_reconstruct(&registry, &disabled, &coordinator, &managed_git);
    tokio::pin!(reconciliation);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut reconciliation)
            .await
            .is_err(),
        "disable waits only for the Vault's active work"
    );
    assert_eq!(
        coordinator.request(vault_id, VaultWorkKind::Index),
        ScheduleResult::Rejected
    );

    release.notify_one();
    reconciliation.await;
    running
        .await
        .expect("worker task")
        .expect("active work completes")
        .result
        .expect("active work succeeds");
    assert!(collection.runtime(vault_id).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabling_after_an_admitted_index_retires_its_late_publication() {
    let directory = tempdir().expect("temporary state directory");
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    for path in [&first_path, &second_path] {
        std::fs::create_dir_all(path).expect("Vault directory");
        std::fs::write(path.join("Home.md"), "# Home\n\nold").expect("Vault note");
    }
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let one = add_local_vault(&registry, &empty, "First", first_path.clone());
    let both = add_local_vault(&registry, &one, "Second", second_path.clone());
    let first = vault_id_named(&both, "First");
    let second = vault_id_named(&both, "Second");
    let cache = Arc::new(SqliteCache::in_memory(384).expect("cache"));
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
    for (id, path) in [(first, &first_path), (second, &second_path)] {
        cache
            .replace_vault_snapshot(
                id,
                &crate::vault::VaultIndex::build(path).expect("index"),
                embedder.as_ref(),
            )
            .expect("publish");
    }
    let collection = VaultCollectionRuntime::with_watching_and_cache(
        directory.path().join("cache.sqlite3"),
        cache.clone(),
    );
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = Arc::new(ManagedGitScheduler::new(coordinator.clone()));
    collection
        .reconcile_and_reconstruct(&registry, &both, &coordinator, &managed_git)
        .await;
    // Drain reconstruction requests; snapshots above are the retained baseline.
    for _ in [first, second] {
        worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .expect("turn");
    }
    std::fs::write(first_path.join("Home.md"), "# Home\n\nnew").expect("new note");
    coordinator.request(first, VaultWorkKind::Index);
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let running = tokio::spawn({
        let collection = collection.clone();
        let cache = cache.clone();
        let embedder = embedder.clone();
        let started = started.clone();
        let release = release.clone();
        async move {
            let first_collection = collection.clone();
            let first_cache = cache.clone();
            let first_embedder = embedder.clone();
            let outcome = worker
                .run_next(move |request| {
                    let started = started.clone();
                    let release = release.clone();
                    async move {
                        started.notify_one();
                        release.notified().await;
                        dispatch_vault_index_turn(
                            &first_collection,
                            first_cache,
                            first_embedder,
                            request,
                        )
                        .await
                    }
                })
                .await;
            let rerun = worker
                .run_next({
                    let collection = collection.clone();
                    let cache = cache.clone();
                    let embedder = embedder.clone();
                    move |request| async move {
                        dispatch_vault_index_turn(&collection, cache, embedder, request).await
                    }
                })
                .await;
            (worker, outcome, rerun)
        }
    });
    started.notified().await;
    let phase_entered = Arc::new(std::sync::Barrier::new(2));
    let release_phase = Arc::new(std::sync::Barrier::new(2));
    collection.set_after_reconcile_before_drain_hook(Some(Arc::new({
        let phase_entered = phase_entered.clone();
        let release_phase = release_phase.clone();
        move || {
            phase_entered.wait();
            release_phase.wait();
        }
    })));
    let disabled = registry.disable(both.revision(), first).expect("disable");
    let disable_reconcile = tokio::spawn({
        let collection = collection.clone();
        let registry = registry.clone();
        let coordinator = coordinator.clone();
        let managed_git = managed_git.clone();
        let disabled = disabled.clone();
        async move {
            collection
                .reconcile_and_reconstruct(&registry, &disabled, &coordinator, &managed_git)
                .await;
        }
    });
    tokio::task::spawn_blocking({
        let phase_entered = phase_entered.clone();
        move || phase_entered.wait()
    })
    .await
    .expect("disable reaches its reconcile phase");
    collection.set_after_reconcile_before_drain_hook(None);
    let enabled = registry.enable(disabled.revision(), first).expect("enable");
    let mut enable_reconcile = tokio::spawn({
        let collection = collection.clone();
        let registry = registry.clone();
        let coordinator = coordinator.clone();
        let managed_git = managed_git.clone();
        async move {
            collection
                .reconcile_and_reconstruct(&registry, &enabled, &coordinator, &managed_git)
                .await;
        }
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut enable_reconcile,)
            .await
            .is_err(),
        "newer enable waits until the older reconcile finishes its drain phase"
    );
    tokio::task::spawn_blocking(move || release_phase.wait())
        .await
        .expect("release disable reconcile phase");
    release.notify_one();
    disable_reconcile.await.expect("disable reconciliation");
    enable_reconcile.await.expect("enable reconciliation");
    let (_worker, outcome, rerun) = running.await.expect("worker");
    outcome.expect("turn").result.expect("index");
    rerun
        .expect("enabled Index")
        .result
        .expect("enabled publication");
    assert!(
        cache
            .snapshot_status(first)
            .expect("status")
            .expect("snapshot")
            .participating
    );
    assert!(
        cache
            .snapshot_status(second)
            .expect("status")
            .expect("snapshot")
            .participating
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reenable_waits_until_disable_finishes_cache_retirement() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    std::fs::write(vault_path.join("Home.md"), "# Home\n\ncontent").expect("Vault note");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let enabled = add_local_vault(&registry, &empty, "Vault", vault_path.clone());
    let vault_id = vault_id_named(&enabled, "Vault");
    let cache = Arc::new(SqliteCache::in_memory(384).expect("cache"));
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
    cache
        .replace_vault_snapshot(
            vault_id,
            &crate::vault::VaultIndex::build(&vault_path).expect("index"),
            embedder.as_ref(),
        )
        .expect("publish retained snapshot");
    let collection = VaultCollectionRuntime::with_watching_and_cache(
        directory.path().join("cache.sqlite3"),
        cache.clone(),
    );
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = Arc::new(ManagedGitScheduler::new(coordinator.clone()));
    collection
        .reconcile_and_reconstruct(&registry, &enabled, &coordinator, &managed_git)
        .await;
    worker
        .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
        .await
        .expect("initial reconstruction turn");

    let retirement_entered = Arc::new(std::sync::Barrier::new(2));
    let release_retirement = Arc::new(std::sync::Barrier::new(2));
    collection.set_after_post_wait_before_retirement_hook(Some(Arc::new({
        let retirement_entered = retirement_entered.clone();
        let release_retirement = release_retirement.clone();
        move || {
            retirement_entered.wait();
            release_retirement.wait();
        }
    })));
    let disabled = registry
        .disable(enabled.revision(), vault_id)
        .expect("disable Vault");
    let disable_reconcile = tokio::spawn({
        let collection = collection.clone();
        let registry = registry.clone();
        let coordinator = coordinator.clone();
        let managed_git = managed_git.clone();
        let disabled = disabled.clone();
        async move {
            collection
                .reconcile_and_reconstruct(&registry, &disabled, &coordinator, &managed_git)
                .await;
        }
    });
    tokio::task::spawn_blocking({
        let retirement_entered = retirement_entered.clone();
        move || retirement_entered.wait()
    })
    .await
    .expect("disable reaches cache-retirement phase");
    collection.set_after_post_wait_before_retirement_hook(None);

    let reenabled = registry
        .enable(disabled.revision(), vault_id)
        .expect("re-enable Vault");
    let mut enable_reconcile = tokio::spawn({
        let collection = collection.clone();
        let registry = registry.clone();
        let coordinator = coordinator.clone();
        let managed_git = managed_git.clone();
        async move {
            collection
                .reconcile_and_reconstruct(&registry, &reenabled, &coordinator, &managed_git)
                .await;
        }
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut enable_reconcile)
            .await
            .is_err(),
        "re-enable waits until disable completes cache retirement"
    );
    tokio::task::spawn_blocking(move || release_retirement.wait())
        .await
        .expect("release cache-retirement phase");
    disable_reconcile.await.expect("disable reconciliation");
    enable_reconcile.await.expect("enable reconciliation");

    worker
        .run_next({
            let collection = collection.clone();
            let cache = cache.clone();
            let embedder = embedder.clone();
            move |request| async move {
                dispatch_vault_index_turn(&collection, cache, embedder, request).await
            }
        })
        .await
        .expect("re-enabled Index turn")
        .result
        .expect("re-enabled publication");
    assert!(collection.runtime(vault_id).is_some());
    assert!(
        cache
            .snapshot_status(vault_id)
            .expect("snapshot status")
            .expect("snapshot")
            .participating
    );
}

#[tokio::test]
async fn disconnecting_after_an_admitted_index_deletes_its_late_publication() {
    let directory = tempdir().expect("temporary state directory");
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    for path in [&first_path, &second_path] {
        std::fs::create_dir_all(path).expect("Vault directory");
        std::fs::write(path.join("Home.md"), "# Home\n\nold").expect("Vault note");
    }
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let one = add_local_vault(&registry, &empty, "First", first_path.clone());
    let both = add_local_vault(&registry, &one, "Second", second_path.clone());
    let first = vault_id_named(&both, "First");
    let second = vault_id_named(&both, "Second");
    let cache = Arc::new(SqliteCache::in_memory(384).expect("cache"));
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
    for (id, path) in [(first, &first_path), (second, &second_path)] {
        cache
            .replace_vault_snapshot(
                id,
                &crate::vault::VaultIndex::build(path).expect("index"),
                embedder.as_ref(),
            )
            .expect("publish");
    }
    let collection = VaultCollectionRuntime::with_watching_and_cache(
        directory.path().join("cache.sqlite3"),
        cache.clone(),
    );
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &both, &coordinator, &managed_git)
        .await;
    for _ in [first, second] {
        worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .expect("turn");
    }
    coordinator.request(first, VaultWorkKind::Index);
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let running = tokio::spawn({
        let collection = collection.clone();
        let cache = cache.clone();
        let embedder = embedder.clone();
        let started = started.clone();
        let release = release.clone();
        async move {
            worker
                .run_next(move |request| {
                    let started = started.clone();
                    let release = release.clone();
                    async move {
                        started.notify_one();
                        release.notified().await;
                        dispatch_vault_index_turn(&collection, cache, embedder, request).await
                    }
                })
                .await
        }
    });
    started.notified().await;
    let disconnected = registry
        .disconnect(both.revision(), first)
        .expect("disconnect");
    let reconcile =
        collection.reconcile_and_reconstruct(&registry, &disconnected, &coordinator, &managed_git);
    tokio::pin!(reconcile);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut reconcile)
            .await
            .is_err()
    );
    release.notify_one();
    reconcile.await;
    running
        .await
        .expect("worker")
        .expect("turn")
        .result
        .expect("index");
    assert_eq!(cache.snapshot_status(first).expect("status"), None);
    assert_eq!(cache.snapshot_note_count(first).expect("rows"), 0);
    assert!(
        cache
            .snapshot_status(second)
            .expect("status")
            .expect("snapshot")
            .participating
    );
}

#[tokio::test]
async fn disconnecting_a_disabled_vault_deletes_its_retained_snapshot() {
    let directory = tempdir().expect("temporary state directory");
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    for path in [&first_path, &second_path] {
        std::fs::create_dir_all(path).expect("Vault directory");
        std::fs::write(path.join("Home.md"), "# Home\n\ncontent").expect("Vault note");
    }
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let one = add_local_vault(&registry, &empty, "First", first_path.clone());
    let both = add_local_vault(&registry, &one, "Second", second_path.clone());
    let first = vault_id_named(&both, "First");
    let second = vault_id_named(&both, "Second");
    let cache = Arc::new(SqliteCache::in_memory(384).expect("cache"));
    let embedder = StubEmbedder::new(384);
    for (id, path) in [(first, &first_path), (second, &second_path)] {
        cache
            .replace_vault_snapshot(
                id,
                &crate::vault::VaultIndex::build(path).expect("index"),
                &embedder,
            )
            .expect("publish");
    }
    let collection = VaultCollectionRuntime::with_watching_and_cache(
        directory.path().join("cache.sqlite3"),
        cache.clone(),
    );
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &both, &coordinator, &managed_git)
        .await;
    let disabled = registry.disable(both.revision(), first).expect("disable");
    collection
        .reconcile_and_reconstruct(&registry, &disabled, &coordinator, &managed_git)
        .await;
    assert!(
        !cache
            .snapshot_status(first)
            .expect("status")
            .expect("snapshot")
            .participating
    );
    let disconnected = registry
        .disconnect(disabled.revision(), first)
        .expect("disconnect");
    collection
        .reconcile_and_reconstruct(&registry, &disconnected, &coordinator, &managed_git)
        .await;
    assert_eq!(cache.snapshot_status(first).expect("status"), None);
    assert_eq!(cache.snapshot_note_count(first).expect("rows"), 0);
    assert!(
        cache
            .snapshot_status(second)
            .expect("status")
            .expect("snapshot")
            .participating
    );
}

#[tokio::test]
async fn restart_retries_a_failed_disconnect_retirement() {
    let directory = tempdir().expect("temporary state directory");
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    for path in [&first_path, &second_path] {
        std::fs::create_dir_all(path).expect("dir");
        std::fs::write(path.join("Home.md"), "# Home").expect("note");
    }
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("recovery"),
    };
    let one = add_local_vault(&registry, &empty, "First", first_path.clone());
    let both = add_local_vault(&registry, &one, "Second", second_path.clone());
    let first = vault_id_named(&both, "First");
    let second = vault_id_named(&both, "Second");
    let cache = Arc::new(SqliteCache::in_memory(384).expect("cache"));
    let embedder = StubEmbedder::new(384);
    for (id, path) in [(first, &first_path), (second, &second_path)] {
        cache
            .replace_vault_snapshot(
                id,
                &crate::vault::VaultIndex::build(path).expect("index"),
                &embedder,
            )
            .expect("publish");
    }
    let collection = VaultCollectionRuntime::with_watching_and_cache(
        directory.path().join("cache.sqlite3"),
        cache.clone(),
    );
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &both, &coordinator, &managed)
        .await;
    cache.connection().expect("conn").execute_batch(&format!("CREATE TRIGGER fail_disconnect BEFORE DELETE ON vault_snapshots WHEN OLD.vault_id = '{}' BEGIN SELECT RAISE(ABORT, 'disconnect failed'); END;", first)).expect("trigger");
    let disconnected = registry
        .disconnect(both.revision(), first)
        .expect("disconnect");
    let (sender, receiver) = tokio::sync::oneshot::channel();
    collection
        .reconcile_and_reconstruct_and_wait_for_mutation_boundary(
            &registry,
            &disconnected,
            &coordinator,
            &managed,
            sender,
        )
        .await;
    assert!(receiver.await.expect("boundary").is_err());
    cache
        .connection()
        .expect("conn")
        .execute_batch("DROP TRIGGER fail_disconnect;")
        .expect("drop trigger");
    let restarted = VaultCollectionRuntime::with_watching_and_cache(
        directory.path().join("restart.sqlite3"),
        cache.clone(),
    );
    let (restart_work, _worker) = VaultWorkCoordinator::new();
    let restart_managed = ManagedGitScheduler::new(restart_work.clone());
    restarted
        .reconcile_and_reconstruct(&registry, &disconnected, &restart_work, &restart_managed)
        .await;
    assert_eq!(cache.snapshot_status(first).expect("status"), None);
    assert_eq!(cache.snapshot_note_count(first).expect("rows"), 0);
    assert!(
        cache
            .snapshot_status(second)
            .expect("status")
            .expect("snapshot")
            .participating
    );
}

#[tokio::test]
async fn disable_reports_a_target_scoped_snapshot_retirement_failure() {
    let directory = tempdir().expect("temporary state directory");
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    for path in [&first_path, &second_path] {
        std::fs::create_dir_all(path).expect("dir");
        std::fs::write(path.join("Home.md"), "# Home").expect("note");
    }
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("recovery"),
    };
    let one = add_local_vault(&registry, &empty, "First", first_path.clone());
    let both = add_local_vault(&registry, &one, "Second", second_path.clone());
    let first = vault_id_named(&both, "First");
    let second = vault_id_named(&both, "Second");
    let cache = Arc::new(SqliteCache::in_memory(384).expect("cache"));
    let embedder = StubEmbedder::new(384);
    for (id, path) in [(first, &first_path), (second, &second_path)] {
        cache
            .replace_vault_snapshot(
                id,
                &crate::vault::VaultIndex::build(path).expect("index"),
                &embedder,
            )
            .expect("publish");
    }
    let collection = VaultCollectionRuntime::with_watching_and_cache(
        directory.path().join("cache.sqlite3"),
        cache.clone(),
    );
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &both, &coordinator, &managed)
        .await;
    cache.connection().expect("conn").execute_batch(&format!("CREATE TRIGGER fail_disable BEFORE UPDATE OF participating ON vault_snapshots WHEN OLD.vault_id = '{}' BEGIN SELECT RAISE(ABORT, 'injected retirement failure'); END;", first)).expect("trigger");
    let disabled = registry
        .disable(both.revision(), first)
        .expect("disable committed");
    let (sender, receiver) = tokio::sync::oneshot::channel();
    collection
        .reconcile_and_reconstruct_and_wait_for_mutation_boundary(
            &registry,
            &disabled,
            &coordinator,
            &managed,
            sender,
        )
        .await;
    assert!(receiver.await.expect("boundary").is_err());
    assert!(!collection.snapshot().vaults[&first].enabled);
    assert!(
        cache
            .snapshot_status(second)
            .expect("status")
            .expect("snapshot")
            .participating
    );
    cache
        .connection()
        .expect("conn")
        .execute_batch("DROP TRIGGER fail_disable;")
        .expect("drop trigger");
    let (retry_sender, retry_receiver) = tokio::sync::oneshot::channel();
    collection
        .reconcile_and_reconstruct_and_wait_for_mutation_boundary(
            &registry,
            &disabled,
            &coordinator,
            &managed,
            retry_sender,
        )
        .await;
    assert!(retry_receiver.await.expect("retry boundary").is_ok());
    assert!(
        !cache
            .snapshot_status(first)
            .expect("status")
            .expect("snapshot")
            .participating
    );
    let enabled = registry
        .enable(disabled.revision(), first)
        .expect("enable committed");
    collection
        .reconcile_and_reconstruct(&registry, &enabled, &coordinator, &managed)
        .await;
    assert!(
        !cache
            .snapshot_status(first)
            .expect("status")
            .expect("snapshot")
            .participating,
        "re-enable must wait for a successful new Index publication"
    );
}

#[tokio::test]
async fn replacing_an_enabled_vault_waits_for_old_work_then_reconstructs_new_work() {
    use std::sync::Arc;

    use crate::vault_work::{ScheduleResult, VaultWorkCoordinator, VaultWorkError};

    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let enabled = add_local_vault(&registry, &empty, "Vault", vault_path.clone());
    let vault_id = vault_id_named(&enabled, "Vault");
    let collection = VaultCollectionRuntime::new();
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &enabled, &coordinator, &managed_git)
        .await;

    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let running = tokio::spawn({
        let started = started.clone();
        let release = release.clone();
        async move {
            let outcome = worker
                .run_next(move |request| {
                    let started = started.clone();
                    let release = release.clone();
                    async move {
                        assert_eq!(request.vault_id(), vault_id);
                        started.notify_one();
                        release.notified().await;
                        Ok::<(), VaultWorkError>(())
                    }
                })
                .await;
            (worker, outcome)
        }
    });
    started.notified().await;

    let replacement = registry
        .edit(
            enabled.revision(),
            vault_id,
            VaultDefinitionEdit {
                name: "Vault".to_string(),
                source: RegistryVaultSource::Local { path: vault_path },
                exclude_patterns: vec!["ignored/**".to_string()],
                https_credentials: HttpsCredentialUpdate::Keep,
                confirm_identity_change: false,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("replace enabled Vault definition");
    let reconciliation =
        collection.reconcile_and_reconstruct(&registry, &replacement, &coordinator, &managed_git);
    tokio::pin!(reconciliation);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut reconciliation)
            .await
            .is_err(),
        "replacement waits for the old control block's active work"
    );
    assert_eq!(
        coordinator.request(vault_id, VaultWorkKind::Index),
        ScheduleResult::Rejected
    );

    release.notify_one();
    reconciliation.await;
    let (mut worker, old_outcome) = running.await.expect("worker task");
    old_outcome
        .expect("old active work completes")
        .result
        .expect("old active work succeeds");
    let replacement_turn = worker
        .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
        .await
        .expect("replacement work reconstructed");
    assert_eq!(replacement_turn.request.vault_id(), vault_id);
    assert_eq!(replacement_turn.request.kind(), VaultWorkKind::Index);
}

#[tokio::test]
async fn disconnecting_a_vault_discards_its_work_without_delaying_another_vault() {
    use crate::vault_work::{ScheduleResult, VaultWorkCoordinator, VaultWorkError};

    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let target_path = directory.path().join("target");
    let healthy_path = directory.path().join("healthy");
    std::fs::create_dir_all(&target_path).expect("target Vault directory");
    std::fs::create_dir_all(&healthy_path).expect("healthy Vault directory");
    let target = add_local_vault(&registry, &empty, "Target", target_path);
    let both = add_local_vault(&registry, &target, "Healthy", healthy_path);
    let target_id = vault_id_named(&both, "Target");
    let healthy_id = vault_id_named(&both, "Healthy");
    let collection = VaultCollectionRuntime::new();
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &both, &coordinator, &managed_git)
        .await;

    let disconnected = registry
        .disconnect(both.revision(), target_id)
        .expect("disconnect target Vault");
    collection
        .reconcile_and_reconstruct(&registry, &disconnected, &coordinator, &managed_git)
        .await;

    assert!(collection.runtime(target_id).is_none());
    assert_eq!(
        coordinator.request(target_id, VaultWorkKind::Index),
        ScheduleResult::Rejected
    );
    let healthy_turn = worker
        .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
        .await
        .expect("healthy Vault work remains runnable");
    assert_eq!(healthy_turn.request.vault_id(), healthy_id);
    assert_eq!(healthy_turn.request.kind(), VaultWorkKind::Index);
}

#[tokio::test]
async fn graceful_shutdown_revokes_vaults_and_discards_reconstructible_work() {
    use std::sync::Arc;

    use crate::vault_work::{ScheduleResult, VaultWorkCoordinator, VaultWorkError, VaultWorkKind};

    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let snapshot = add_local_vault(&registry, &empty, "Vault", vault_path);
    let vault_id = vault_id_named(&snapshot, "Vault");
    let collection = VaultCollectionRuntime::new();
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &snapshot, &coordinator, &managed_git)
        .await;
    let runtime = collection.runtime(vault_id).expect("active runtime");
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let running = tokio::spawn({
        let started = started.clone();
        let release = release.clone();
        async move {
            let outcome = worker
                .run_next(move |request| {
                    let started = started.clone();
                    let release = release.clone();
                    async move {
                        assert_eq!(request.vault_id(), vault_id);
                        started.notify_one();
                        release.notified().await;
                        Ok::<(), VaultWorkError>(())
                    }
                })
                .await;
            (worker, outcome)
        }
    });
    started.notified().await;
    assert_eq!(
        coordinator.request(vault_id, VaultWorkKind::Git),
        ScheduleResult::Queued
    );

    let mut shutdown = Box::pin(tokio::spawn({
        let collection = collection.clone();
        let coordinator = coordinator.clone();
        async move { collection.shutdown(&coordinator).await }
    }));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut shutdown)
            .await
            .is_err(),
        "shutdown waits for active work instead of draining the queued rerun"
    );

    release.notify_one();
    shutdown.await.expect("shutdown task");
    let (mut worker, active_outcome) = running.await.expect("worker task");
    active_outcome
        .expect("active work completes")
        .result
        .expect("active work succeeds");

    assert!(!runtime.is_accepting_operations());
    assert_eq!(
        coordinator.request(vault_id, VaultWorkKind::Index),
        ScheduleResult::Rejected
    );
    assert!(
        worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .is_none(),
        "queued work is reconstructed after restart instead of delaying shutdown"
    );
}

/// Restart reconstruction must arm managed-Git polling, not just Vault
/// activation: a Vault reconstructed from an existing on-disk registry is
/// registered with `ManagedGitScheduler` and due immediately, so
/// `spawn_scheduler_tick`'s first tick runs an initial sync and every later
/// re-arm keeps it on its configured interval. Nothing covered this before:
/// the scheduler's own tests activate it by hand, and
/// `vault_management.rs`'s cover only the *create* path, so a restart —
/// the one path every deployment takes on every deploy — reached the
/// scheduler through an untested branch of the activation loop.
///
/// A `Local` Vault in the same collection proves the other half: a source
/// with no remote is never tracked, so reconstruction cannot start polling
/// something that has nothing to poll.
#[tokio::test]
async fn restart_reconstruction_arms_managed_git_polling_and_leaves_local_vaults_alone() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let local_path = directory.path().join("local");
    std::fs::create_dir_all(&local_path).expect("local Vault directory");
    let one = add_local_vault(&registry, &empty, "Local notes", local_path);
    let committed = registry
        .add(
            one.revision(),
            NewVaultDefinition {
                name: "Managed".to_string(),
                enabled: true,
                source: RegistryVaultSource::ManagedGit {
                    repository_url: "https://example.test/vault.git".to_string(),
                    branch: Some("main".to_string()),
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
    let managed_vault = vault_id_named(&committed, "Managed");
    let local_vault = vault_id_named(&committed, "Local notes");
    // The checkout this Vault already had before the restart.
    let vault_path = registry.vault_path(
        &committed
            .definitions()
            .find(|definition| definition.vault_id() == managed_vault)
            .expect("managed Vault definition"),
    );
    std::fs::create_dir_all(&vault_path).expect("existing checkout");
    std::fs::write(vault_path.join("Home.md"), "# Home").expect("note");

    // A fresh process: new collection runtime, coordinator, and scheduler,
    // reconstructing from the registry exactly as `server::run_server` does.
    let collection = VaultCollectionRuntime::new();
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    let reloaded = match registry.load().expect("reload registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    collection
        .reconcile_and_reconstruct(&registry, &reloaded, &coordinator, &managed_git)
        .await;

    assert_eq!(
        managed_git.poll_interval_for_test(managed_vault),
        Some(std::time::Duration::from_secs(
            DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS
        )),
        "restart reconstruction must register a managed-Git Vault with the scheduler"
    );
    assert!(
        managed_git
            .next_attempt_for_test(managed_vault)
            .expect("armed schedule")
            <= std::time::Instant::now(),
        "a reconstructed managed-Git Vault must be due immediately, not one interval out"
    );
    assert_eq!(
        managed_git.poll_interval_for_test(local_vault),
        None,
        "a Local Vault has no remote and must never be scheduled"
    );
}

/// Disconnecting a Vault removes it from the collection entirely, so the
/// durable record of its Git turns must go with it — otherwise reconnecting
/// the same Vault ID later would inherit a stale countdown from a Vault that
/// is, as far as the operator is concerned, gone. Disabling deliberately does
/// *not* forget: a Vault that comes back tomorrow should resume its schedule
/// rather than re-sync for having been paused.
#[tokio::test]
async fn disconnecting_a_vault_forgets_its_remembered_git_turn() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let committed = registry
        .add(
            empty.revision(),
            NewVaultDefinition {
                name: "Managed".to_string(),
                enabled: true,
                source: RegistryVaultSource::ManagedGit {
                    repository_url: "https://example.test/vault.git".to_string(),
                    branch: Some("main".to_string()),
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
    let vault_id = vault_id_named(&committed, "Managed");
    let store = Arc::new(
        crate::vault_runtime_state::VaultRuntimeStateStore::beside_registry(registry.path()),
    );
    store
        .record_git_turn(
            vault_id,
            crate::vault_runtime_state::GitTurnRecord {
                completed_at: std::time::SystemTime::now(),
                outcome: crate::vault_runtime_state::GitTurnOutcome::UpToDate,
                code: None,
                message: None,
            },
        )
        .expect("remember a turn");

    let collection = VaultCollectionRuntime::new();
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::with_state_store(coordinator.clone(), store.clone());
    collection
        .reconcile_and_reconstruct(&registry, &committed, &coordinator, &managed_git)
        .await;

    let disconnected = registry
        .disconnect(committed.revision(), vault_id)
        .expect("disconnect the Vault");
    collection
        .reconcile_and_reconstruct(&registry, &disconnected, &coordinator, &managed_git)
        .await;

    assert_eq!(
        store.last_git_turn(vault_id),
        None,
        "a disconnected Vault must leave no remembered Git turn behind"
    );
}

/// The behavior the durable schedule exists to deliver, and the one the
/// scheduler alone cannot: reconstruction must not force a Git turn for a
/// Vault that is not due. Activation publishes a `Pending` Git status on
/// every fresh process, and requesting a turn on that alone is what made
/// every restart re-sync — which in turn re-armed the interval from the
/// restart, so a deployment redeployed more often than its poll interval
/// never reached a scheduled turn at all.
#[tokio::test]
async fn restart_reconstruction_does_not_re_sync_a_managed_git_vault_that_is_not_due() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let committed = registry
        .add(
            empty.revision(),
            NewVaultDefinition {
                name: "Managed".to_string(),
                enabled: true,
                source: RegistryVaultSource::ManagedGit {
                    repository_url: "https://example.test/vault.git".to_string(),
                    branch: Some("main".to_string()),
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
    let vault_id = vault_id_named(&committed, "Managed");
    let vault_path = registry.vault_path(
        &committed
            .definitions()
            .find(|definition| definition.vault_id() == vault_id)
            .expect("managed Vault definition"),
    );
    std::fs::create_dir_all(&vault_path).expect("the checkout it already had");
    let store = Arc::new(
        crate::vault_runtime_state::VaultRuntimeStateStore::beside_registry(registry.path()),
    );
    store
        .record_git_turn(
            vault_id,
            crate::vault_runtime_state::GitTurnRecord {
                // An hour ago, against a 24h interval: nowhere near due.
                completed_at: std::time::SystemTime::now() - std::time::Duration::from_secs(3600),
                outcome: crate::vault_runtime_state::GitTurnOutcome::UpToDate,
                code: None,
                message: None,
            },
        )
        .expect("remember the previous process's turn");

    let collection = VaultCollectionRuntime::new();
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::with_state_store(coordinator.clone(), store);
    collection
        .reconcile_and_reconstruct(&registry, &committed, &coordinator, &managed_git)
        .await;

    assert!(
        !coordinator.has_work(vault_id, VaultWorkKind::Git),
        "a Vault an hour into a daily interval must not be re-synced just because Hatchdoor restarted"
    );
}

/// A restart must not make a failing Vault look merely `pending`. Activation
/// publishes `pending` on every fresh process, and now that a restart no
/// longer forces an immediate turn, nothing would re-publish the failure
/// until the Vault's next scheduled turn — up to a full poll interval of a
/// broken Vault reporting nothing wrong. The remembered outcome fills that
/// gap: it is the same failure the previous process published, carried
/// across the restart that would otherwise have erased it.
#[tokio::test]
async fn restart_reconstruction_republishes_a_remembered_git_failure() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let committed = registry
        .add(
            empty.revision(),
            NewVaultDefinition {
                name: "Managed".to_string(),
                enabled: true,
                source: RegistryVaultSource::ManagedGit {
                    repository_url: "https://example.test/vault.git".to_string(),
                    branch: Some("main".to_string()),
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
    let vault_id = vault_id_named(&committed, "Managed");
    let vault_path = registry.vault_path(
        &committed
            .definitions()
            .find(|definition| definition.vault_id() == vault_id)
            .expect("managed Vault definition"),
    );
    std::fs::create_dir_all(&vault_path).expect("the checkout it already had");
    let store = Arc::new(
        crate::vault_runtime_state::VaultRuntimeStateStore::beside_registry(registry.path()),
    );
    store
        .record_git_turn(
            vault_id,
            crate::vault_runtime_state::GitTurnRecord {
                completed_at: std::time::SystemTime::now() - std::time::Duration::from_secs(3600),
                outcome: crate::vault_runtime_state::GitTurnOutcome::Failed,
                code: Some("managed_git_authentication_failed".to_string()),
                message: Some("the remote rejected the stored token".to_string()),
            },
        )
        .expect("remember the failure the previous process published");

    let collection = VaultCollectionRuntime::new();
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::with_state_store(coordinator.clone(), store);
    collection
        .reconcile_and_reconstruct(&registry, &committed, &coordinator, &managed_git)
        .await;

    let snapshot = collection.snapshot();
    let vault = &snapshot.vaults[&vault_id];
    assert_eq!(
        vault.git,
        VaultGitStatus::Unavailable,
        "a Vault whose last turn failed must not report `pending` after a restart"
    );
    let error = vault.git_error.as_ref().expect("the remembered failure");
    assert_eq!(error.code, "managed_git_authentication_failed");
    assert_eq!(error.message, "the remote rejected the stored token");
    assert!(
        !error.retryable,
        "only non-retryable outcomes are remembered"
    );
}

/// The same carry-across applies to a healthy Vault: one whose last turn
/// succeeded reports `ready` after a restart rather than `pending`, so a
/// Vault that is simply waiting out its interval does not look like one
/// still working through its first sync.
#[tokio::test]
async fn restart_reconstruction_republishes_a_remembered_git_success() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let committed = registry
        .add(
            empty.revision(),
            NewVaultDefinition {
                name: "Managed".to_string(),
                enabled: true,
                source: RegistryVaultSource::ManagedGit {
                    repository_url: "https://example.test/vault.git".to_string(),
                    branch: Some("main".to_string()),
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
    let vault_id = vault_id_named(&committed, "Managed");
    let vault_path = registry.vault_path(
        &committed
            .definitions()
            .find(|definition| definition.vault_id() == vault_id)
            .expect("managed Vault definition"),
    );
    std::fs::create_dir_all(&vault_path).expect("the checkout it already had");
    let store = Arc::new(
        crate::vault_runtime_state::VaultRuntimeStateStore::beside_registry(registry.path()),
    );
    store
        .record_git_turn(
            vault_id,
            crate::vault_runtime_state::GitTurnRecord {
                completed_at: std::time::SystemTime::now() - std::time::Duration::from_secs(3600),
                outcome: crate::vault_runtime_state::GitTurnOutcome::Synchronized,
                code: None,
                message: None,
            },
        )
        .expect("remember a healthy turn");

    let collection = VaultCollectionRuntime::new();
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::with_state_store(coordinator.clone(), store);
    collection
        .reconcile_and_reconstruct(&registry, &committed, &coordinator, &managed_git)
        .await;

    let snapshot = collection.snapshot();
    let vault = &snapshot.vaults[&vault_id];
    assert_eq!(vault.git, VaultGitStatus::Ready);
    assert!(vault.git_error.is_none());
}
