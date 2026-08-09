use super::*;
use tempfile::tempdir;

use crate::cache::SqliteCache;
use crate::cache::vault_snapshots::{VaultSnapshotFreshness, VaultSnapshotStatus};
use crate::embed::{Embedder, StubEmbedder};
use crate::vault_registry::{
    HttpsCredentialUpdate, NewVaultDefinition, VaultDefinitionEdit, VaultGitMode,
    VaultRegistrySnapshot, VaultRegistryStore, VaultSource as RegistryVaultSource,
};

struct PanicEmbedder;

impl Embedder for PanicEmbedder {
    fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        panic!("test candidate task panic");
    }

    fn embedding_dim(&self) -> usize {
        384
    }

    fn token_count(&self, _text: &str, _add_special_tokens: bool) -> Result<usize, String> {
        Ok(1)
    }
}

fn managed(mode: ManagedGitMode) -> VaultSource {
    VaultSource::ManagedGit(ManagedGitSource {
        repository_url: "https://example.test/vault.git".to_string(),
        checkout_path: PathBuf::from("/data/vault"),
        branch: None,
        vault_subdirectory: None,
        mode,
    })
}

#[test]
fn pull_only_ready_state_never_allows_mutation_or_push() {
    let runtime = VaultRuntime::ready(managed(ManagedGitMode::PullOnly));
    let capabilities = runtime.snapshot().capabilities;
    assert!(capabilities.browse);
    assert!(capabilities.search);
    assert!(capabilities.pull);
    assert!(!capabilities.mutate);
    assert!(!capabilities.push);
}

#[test]
fn unavailable_state_has_no_ready_vault_capabilities() {
    let runtime = VaultRuntime::new(managed(ManagedGitMode::Bidirectional));
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
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

#[tokio::test]
async fn index_turn_publishes_one_vault_and_a_failure_keeps_its_snapshot_stale() {
    let directory = tempdir().expect("temporary state directory");
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    std::fs::create_dir_all(&first_path).expect("create first Vault");
    std::fs::create_dir_all(&second_path).expect("create second Vault");
    std::fs::write(first_path.join("Home.md"), "# Home\n\nfirst version")
        .expect("write first note");
    std::fs::write(second_path.join("Home.md"), "# Home\n\nsecond version")
        .expect("write second note");

    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let one = add_local_vault(&registry, &empty, "First", first_path.clone());
    let both = add_local_vault(&registry, &one, "Second", second_path);
    let first = vault_id_named(&both, "First");
    let second = vault_id_named(&both, "Second");
    let collection = VaultCollectionRuntime::new();
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &both, &coordinator, &managed_git)
        .await;
    let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));
    let working: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));

    for _ in [first, second] {
        let outcome = worker
            .run_next({
                let collection = collection.clone();
                let cache = cache.clone();
                let working = working.clone();
                move |request| async move {
                    dispatch_vault_index_turn(&collection, cache, working, request).await
                }
            })
            .await
            .expect("queued Index turn");
        assert_eq!(outcome.request.kind(), VaultWorkKind::Index);
        outcome.result.expect("Index publication succeeds");
    }

    assert_eq!(
        cache
            .snapshot_note_content(first, "home")
            .expect("read first snapshot")
            .as_deref(),
        Some("# Home\n\nfirst version")
    );
    assert_eq!(
        cache
            .snapshot_note_content(second, "home")
            .expect("read second snapshot")
            .as_deref(),
        Some("# Home\n\nsecond version")
    );

    coordinator.request(second, VaultWorkKind::Index);
    let panicked = worker
        .run_next({
            let collection = collection.clone();
            let cache = cache.clone();
            move |request| async move {
                dispatch_vault_index_turn(&collection, cache, Arc::new(PanicEmbedder), request)
                    .await
            }
        })
        .await
        .expect("panicking candidate turn");
    assert_eq!(panicked.request.vault_id(), second);
    assert_eq!(
        panicked
            .result
            .expect_err("candidate task panic is returned")
            .code(),
        "vault_index_task_panicked"
    );
    assert_eq!(
        cache.snapshot_status(second).expect("read stale status"),
        Some(VaultSnapshotStatus {
            participating: true,
            freshness: VaultSnapshotFreshness::Stale,
        })
    );

    std::fs::remove_dir_all(&first_path).expect("make first Vault unavailable");
    coordinator.request(first, VaultWorkKind::Index);
    coordinator.request(second, VaultWorkKind::Index);
    let failed = worker
        .run_next({
            let collection = collection.clone();
            let cache = cache.clone();
            let working = working.clone();
            move |request| async move {
                dispatch_vault_index_turn(&collection, cache, working, request).await
            }
        })
        .await
        .expect("failing Index turn");
    assert_eq!(failed.request.vault_id(), first);
    assert_eq!(
        failed.result.expect_err("scan failure is returned").code(),
        "vault_index_failed"
    );
    assert_eq!(
        cache
            .snapshot_note_content(first, "home")
            .expect("read retained first snapshot")
            .as_deref(),
        Some("# Home\n\nfirst version")
    );
    assert_eq!(
        collection
            .runtime(first)
            .expect("first runtime")
            .snapshot()
            .search,
        VaultSearchStatus::Stale
    );

    let healthy = worker
        .run_next({
            let collection = collection.clone();
            let cache = cache.clone();
            let working = working.clone();
            move |request| async move {
                dispatch_vault_index_turn(&collection, cache, working, request).await
            }
        })
        .await
        .expect("healthy Vault follows failed turn");
    assert_eq!(healthy.request.vault_id(), second);
    healthy.result.expect("healthy Index succeeds");
    assert_eq!(
        collection
            .runtime(second)
            .expect("second runtime")
            .snapshot()
            .search,
        VaultSearchStatus::Ready
    );
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
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
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
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

/// A managed-Git Vault's control block, activated through the real
/// registry and collection runtime exactly like production. Uses a
/// syntactically valid but unreachable `https://` URL — like
/// `activation_failure_is_isolated_from_healthy_local_markdown` above,
/// the registry only ever accepts credential-free HTTPS, with no test
/// escape (unlike the Git-owned `acquire_or_reuse`/
/// `synchronize_managed_checkout`, which each carry their own
/// `#[cfg(test)]` local-path allowance). `run_managed_git_turn`'s own
/// tests in `git/managed_task.rs` already prove the real `git2`
/// mechanics against a local bare repository; this fixture exists to
/// test `publish_managed_git_turn_outcome`'s status-publishing behavior
/// against a *fabricated* result instead, without a reachable remote.
fn managed_git_control_block(
    directory: &Path,
) -> (
    VaultCollectionRuntime,
    VaultRegistryStore,
    VaultControlBlock,
    VaultId,
) {
    let registry = VaultRegistryStore::new(directory.join("state/vaults.json"));
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
                    repository_url: "https://example.test/vault.git".to_string(),
                    branch: Some("main".to_string()),
                    vault_subdirectory: None,
                    mode: VaultGitMode::PullOnly,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
            },
        )
        .expect("add managed Vault");
    let vault_id = vault_id_named(&committed, "Remote notes");
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &committed);
    let control_block = collection.runtime(vault_id).expect("active runtime");
    (collection, registry, control_block, vault_id)
}

#[tokio::test]
async fn publish_managed_git_turn_outcome_makes_a_successful_vault_ready_and_browsable() {
    let directory = tempdir().expect("temporary state directory");
    let (_collection, _registry, control_block, vault_id) =
        managed_git_control_block(directory.path());
    // The checkout materializes at exactly the path the registry already
    // resolved for this Vault ID — `run_managed_git_turn` installs there
    // in production; this test fabricates that outcome directly.
    std::fs::create_dir_all(control_block.vault_path()).expect("acquired checkout root");
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    managed_git.activate(vault_id);
    assert_eq!(
        control_block.snapshot().local_content,
        LocalContentStatus::Unavailable
    );

    publish_managed_git_turn_outcome(
        &control_block,
        &coordinator,
        &managed_git,
        vault_id,
        &Ok(crate::git::ManagedGitOutcome::Synchronized),
    );

    let after = control_block.snapshot();
    assert_eq!(after.git, VaultGitStatus::Ready);
    assert!(after.git_error.is_none());
    assert_eq!(after.local_content, LocalContentStatus::ReadWrite);
    assert!(after.activation_error.is_none());
    assert!(after.capabilities.browse);
    let index_turn = worker
        .run_next(|request| async move {
            assert_eq!(request.vault_id(), vault_id);
            assert_eq!(request.kind(), VaultWorkKind::Index);
            Ok::<(), VaultWorkError>(())
        })
        .await
        .expect("successful acquisition queues Index work");
    index_turn.result.expect("Index turn can proceed");
}

#[test]
fn publish_managed_git_turn_outcome_isolates_a_failure_from_already_acquired_local_markdown() {
    let directory = tempdir().expect("temporary state directory");
    let (_collection, _registry, control_block, vault_id) =
        managed_git_control_block(directory.path());
    std::fs::create_dir_all(control_block.vault_path()).expect("acquired checkout root");
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    managed_git.activate(vault_id);
    publish_managed_git_turn_outcome(
        &control_block,
        &coordinator,
        &managed_git,
        vault_id,
        &Ok(crate::git::ManagedGitOutcome::UpToDate),
    );
    assert_eq!(
        control_block.snapshot().local_content,
        LocalContentStatus::ReadWrite
    );

    // A later turn fails (e.g. the remote went unreachable). Local
    // Markdown access must not regress just because Git did.
    publish_managed_git_turn_outcome(
        &control_block,
        &coordinator,
        &managed_git,
        vault_id,
        &Err(VaultWorkError::new(
            "managed_git_remote_unreachable",
            "could not resolve host",
            true,
        )),
    );

    let after = control_block.snapshot();
    assert_eq!(after.git, VaultGitStatus::Unavailable);
    assert_eq!(
        after.git_error.as_ref().map(|error| error.code.as_str()),
        Some("managed_git_remote_unreachable")
    );
    assert!(
        after
            .git_error
            .as_ref()
            .is_some_and(|error| error.retryable)
    );
    assert_eq!(
        after.local_content,
        LocalContentStatus::ReadWrite,
        "a Git failure must not revoke already-acquired local Markdown access"
    );
    assert!(after.capabilities.browse);
}

/// Drives a real Git-turn *failure* through the full async dispatch path
/// — credential resolution, `spawn_blocking`, status publishing, and
/// scheduler recording — via `dispatch_managed_git_turn_with`'s injected
/// executor, rather than calling `publish_managed_git_turn_outcome`
/// directly. This is the "not just the generic coordinator mechanism"
/// coverage a real remote failure would exercise, without a reachable
/// remote or a network call in the test suite.
#[tokio::test]
async fn dispatch_managed_git_turn_with_publishes_a_real_failure_through_the_full_async_path() {
    let directory = tempdir().expect("temporary state directory");
    let (collection, registry, control_block, vault_id) =
        managed_git_control_block(directory.path());
    std::fs::create_dir_all(control_block.vault_path()).expect("already-acquired checkout");
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    managed_git.activate(vault_id);

    // First turn succeeds (fabricated), establishing already-acquired
    // local content exactly like a real prior sync would.
    coordinator.request(vault_id, VaultWorkKind::Git);
    worker
        .run_next(|request| {
            dispatch_managed_git_turn_with(
                &collection,
                &registry,
                &coordinator,
                &managed_git,
                "Hatchdoor",
                "hatchdoor@example.test",
                request,
                |_config| Ok(crate::git::ManagedGitOutcome::UpToDate),
            )
        })
        .await
        .expect("first turn dequeued")
        .result
        .expect("first turn succeeds");
    assert_eq!(
        control_block.snapshot().local_content,
        LocalContentStatus::ReadWrite
    );
    let index_turn = worker
        .run_next(|request| async move {
            assert_eq!(request.vault_id(), vault_id);
            assert_eq!(request.kind(), VaultWorkKind::Index);
            Ok::<(), VaultWorkError>(())
        })
        .await
        .expect("successful managed Git turn queues Index work");
    index_turn.result.expect("Index turn can proceed");

    // A later turn fails for real, through the same dispatch path.
    coordinator.request(vault_id, VaultWorkKind::Git);
    let outcome = worker
        .run_next(|request| {
            dispatch_managed_git_turn_with(
                &collection,
                &registry,
                &coordinator,
                &managed_git,
                "Hatchdoor",
                "hatchdoor@example.test",
                request,
                |_config| {
                    Err(VaultWorkError::new(
                        "managed_git_remote_unreachable",
                        "simulated remote outage",
                        true,
                    ))
                },
            )
        })
        .await
        .expect("Git turn dequeued");

    assert_eq!(outcome.request.vault_id(), vault_id);
    assert_eq!(outcome.request.kind(), VaultWorkKind::Git);
    let error = outcome.result.expect_err("injected failure propagates");
    assert_eq!(error.code(), "managed_git_remote_unreachable");
    assert!(error.retryable());

    let after = control_block.snapshot();
    assert_eq!(after.git, VaultGitStatus::Unavailable);
    assert_eq!(
        after.git_error.as_ref().map(|error| error.code.as_str()),
        Some("managed_git_remote_unreachable")
    );
    assert_eq!(
        after.local_content,
        LocalContentStatus::ReadWrite,
        "already-acquired local Markdown must survive a real dispatched failure"
    );
    assert!(after.capabilities.browse);

    // The failure also released the worker: a healthy Vault's turn can
    // still proceed right after, through the very same worker.
    let current = match registry.load().expect("load registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => {
            panic!("registry recovery")
        }
    };
    let healthy_path = directory.path().join("healthy");
    std::fs::create_dir_all(&healthy_path).expect("healthy Vault directory");
    let updated = add_local_vault(&registry, &current, "Healthy local", healthy_path);
    let healthy = vault_id_named(&updated, "Healthy local");
    coordinator.request(healthy, VaultWorkKind::Repair);
    let healthy_turn = worker
        .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
        .await
        .expect("worker still runs turns after the failure");
    assert_eq!(healthy_turn.request.vault_id(), healthy);
}

#[tokio::test]
async fn dispatch_managed_git_turn_is_a_no_op_for_a_non_managed_git_vault() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let one = add_local_vault(&registry, &empty, "Local Vault", vault_path);
    let vault_id = vault_id_named(&one, "Local Vault");
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &one);
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    // A Local Vault's Git status is `Disabled`, never `Pending`, so
    // nothing would request Git work for it in production; requesting it
    // directly here exercises `dispatch_managed_git_turn`'s defensive
    // non-managed-Git branch without any real Git I/O.
    coordinator.request(vault_id, VaultWorkKind::Git);

    let outcome = worker
        .run_next(|request| {
            dispatch_managed_git_turn(
                &collection,
                &registry,
                &coordinator,
                &managed_git,
                "Hatchdoor",
                "hatchdoor@example.test",
                request,
            )
        })
        .await
        .expect("turn dequeued");

    outcome
        .result
        .expect("non-managed-Git dispatch is a harmless no-op");
    let snapshot = collection
        .runtime(vault_id)
        .expect("active runtime")
        .snapshot();
    assert_eq!(snapshot.git, VaultGitStatus::Disabled);
    assert!(snapshot.git_error.is_none());
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
