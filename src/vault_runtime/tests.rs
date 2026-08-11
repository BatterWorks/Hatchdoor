use super::*;
use tempfile::tempdir;

use crate::cache::SqliteCache;
use crate::cache::vault_snapshots::{VaultSnapshotFreshness, VaultSnapshotStatus};
use crate::embed::{Embedder, StubEmbedder};
use crate::vault_registry::{
    DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS, HttpsCredentialUpdate, NewVaultDefinition,
    VaultDefinitionEdit, VaultGitMode, VaultRegistrySnapshot, VaultRegistryStore,
    VaultSource as RegistryVaultSource,
};

struct BlockingEmbedder {
    inner: StubEmbedder,
    entered: Arc<std::sync::Barrier>,
    release: Arc<std::sync::Barrier>,
}

impl Embedder for BlockingEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        self.entered.wait();
        self.release.wait();
        self.inner.embed(texts)
    }

    fn embedding_dim(&self) -> usize {
        self.inner.embedding_dim()
    }

    fn identity(&self) -> String {
        self.inner.identity()
    }

    fn token_count(&self, text: &str, add_special_tokens: bool) -> Result<usize, String> {
        self.inner.token_count(text, add_special_tokens)
    }
}

struct PanicEmbedder;

impl Embedder for PanicEmbedder {
    fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        panic!("test candidate task panic");
    }

    fn embedding_dim(&self) -> usize {
        384
    }

    fn identity(&self) -> String {
        "stub-384".to_string()
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

/// Regression for #99's reopening: an Index turn set only the runtime search
/// status to `Indexing`, but every collection-shaped read (`VaultReadCore`'s
/// `collection` helper backing tree/stats/graph/recent, and
/// `VaultSearchCore::search`) derives participant freshness solely from the
/// cache-published `VaultSnapshotStatus`, which stayed `Fresh` throughout the
/// authoritative scan/candidate build. This held the turn open mid-build with
/// a blocking embedder and asserted a concurrent collection read observed the
/// indexing lag explicitly instead of a silently fresh retained snapshot.
#[tokio::test]
async fn active_index_turn_reports_the_retained_snapshot_stale_to_concurrent_reads() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("create Vault directory");
    std::fs::write(vault_path.join("Home.md"), "# Home\n\noriginal").expect("write note");

    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let snapshot = add_local_vault(&registry, &empty, "Only", vault_path.clone());
    let vault_id = vault_id_named(&snapshot, "Only");

    let collection = VaultCollectionRuntime::new();
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &snapshot, &coordinator, &managed_git)
        .await;
    let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));
    let working: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));

    let published = worker
        .run_next({
            let collection = collection.clone();
            let cache = cache.clone();
            let working = working.clone();
            move |request| async move {
                dispatch_vault_index_turn(&collection, cache, working, request).await
            }
        })
        .await
        .expect("initial Index turn");
    published.result.expect("initial publication succeeds");
    assert_eq!(
        cache.snapshot_status(vault_id).expect("read status"),
        Some(VaultSnapshotStatus {
            participating: true,
            freshness: VaultSnapshotFreshness::Fresh,
        }),
        "initial publish is fresh"
    );

    std::fs::write(vault_path.join("Home.md"), "# Home\n\nupdated").expect("update note");
    coordinator.request(vault_id, VaultWorkKind::Index);

    let entered = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    let blocking_embedder: Arc<dyn Embedder> = Arc::new(BlockingEmbedder {
        inner: StubEmbedder::new(384),
        entered: entered.clone(),
        release: release.clone(),
    });

    let active = tokio::spawn({
        let collection = collection.clone();
        let cache = cache.clone();
        async move {
            worker
                .run_next(move |request| {
                    let collection = collection.clone();
                    let cache = cache.clone();
                    async move {
                        dispatch_vault_index_turn(&collection, cache, blocking_embedder, request)
                            .await
                    }
                })
                .await
        }
    });

    tokio::task::spawn_blocking({
        let entered = entered.clone();
        move || entered.wait()
    })
    .await
    .expect("wait for candidate build to begin");

    // Assertions run while `BlockingEmbedder` still holds a blocking-pool
    // thread parked on `release.wait()`. A bare panic here would unwind the
    // `#[tokio::test]` runtime before that thread's barrier party ever
    // arrives, and dropping a Tokio runtime blocks indefinitely for
    // outstanding blocking tasks — so the test would hang instead of
    // reporting the failure. Always release the barrier first, then resume
    // any panic so the assertion failure still surfaces normally.
    let mid_rebuild_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_eq!(
            collection
                .runtime(vault_id)
                .expect("active runtime")
                .snapshot()
                .search,
            VaultSearchStatus::Indexing,
            "runtime status reflects the active turn"
        );
        assert_eq!(
            cache.snapshot_status(vault_id).expect("read status"),
            Some(VaultSnapshotStatus {
                participating: true,
                freshness: VaultSnapshotFreshness::Stale,
            }),
            "the retained snapshot must not read as fresh while its replacement is being built"
        );

        let projection = crate::vault_read::VaultReadCore::new(&cache, &collection)
            .trees(crate::vault_read::VaultScope::One(vault_id))
            .expect("tree read during active rebuild");
        assert!(
            projection.partial,
            "a collection read during an active rebuild must report partial"
        );
        assert_eq!(
            projection.participants[0].state,
            crate::vault_read::VaultParticipantState::Stale,
            "indexing lag must be explicit to collection-shaped reads, not silently fresh"
        );
    }));

    tokio::task::spawn_blocking({
        let release = release.clone();
        move || release.wait()
    })
    .await
    .expect("release candidate build");

    if let Err(panic) = mid_rebuild_result {
        std::panic::resume_unwind(panic);
    }

    let outcome = active.await.expect("worker task").expect("Index turn ran");
    outcome.result.expect("rebuild publishes successfully");

    assert_eq!(
        cache.snapshot_status(vault_id).expect("read status"),
        Some(VaultSnapshotStatus {
            participating: true,
            freshness: VaultSnapshotFreshness::Fresh,
        }),
        "a successful rebuild republishes fresh"
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
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
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
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
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
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
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
    managed_git.activate(
        vault_id,
        std::time::Duration::from_secs(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS),
    );
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
    managed_git.activate(
        vault_id,
        std::time::Duration::from_secs(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS),
    );
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
    managed_git.activate(
        vault_id,
        std::time::Duration::from_secs(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS),
    );

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
                |_config, _lease| Ok(crate::git::ManagedGitOutcome::UpToDate),
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
                |_config, _lease| {
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

/// Closes issue #96's reopening defect 2: `dispatch_managed_git_turn_with`
/// used to run its blocking `git2` turn without ever acquiring
/// `VaultControlBlock::mutation_lock`, so a foreground Markdown write (which
/// acquires that same lock — see `handlers::vault_write::acquire_mutation`
/// and `mcp::tools::write::acquire_mutation`) could race a Git turn's
/// fetch/integrate/reset phases.
///
/// Proves the fix by acquiring the mutation lock directly in the test —
/// simulating a foreground write already in flight — then driving a real
/// managed-Git turn (via `dispatch_managed_git_turn_with`'s injected
/// executor, so no reachable remote is needed) through the same worker.
/// Before defect 2's fix, the dispatch path never awaited the lock at all,
/// so the turn would race straight through even while the guard below is
/// held, and the first assertion below would fail (the turn would resolve
/// well inside the 200ms window instead of timing out).
#[tokio::test]
async fn a_managed_git_turn_waits_for_a_concurrent_foreground_mutation_to_release_the_lock() {
    let directory = tempdir().expect("temporary state directory");
    let (collection, registry, control_block, vault_id) =
        managed_git_control_block(directory.path());
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    managed_git.activate(
        vault_id,
        std::time::Duration::from_secs(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS),
    );
    coordinator.request(vault_id, VaultWorkKind::Git);

    // Simulate a foreground Markdown write already in flight, holding
    // exactly the lock a real write handler acquires.
    let mutation_guard = control_block
        .acquire_mutation()
        .await
        .expect("foreground mutation lock");

    let dispatch = worker.run_next(|request| {
        dispatch_managed_git_turn_with(
            &collection,
            &registry,
            &coordinator,
            &managed_git,
            "Hatchdoor",
            "hatchdoor@example.test",
            request,
            |_config, _lease| Ok(crate::git::ManagedGitOutcome::UpToDate),
        )
    });
    tokio::pin!(dispatch);

    let raced = tokio::time::timeout(std::time::Duration::from_millis(200), &mut dispatch).await;
    assert!(
        raced.is_err(),
        "the Git turn must block on the foreground mutation lock, not race past it"
    );

    drop(mutation_guard);

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), dispatch)
        .await
        .expect("Git turn proceeds once the foreground mutation releases the lock")
        .expect("Git turn dequeued");
    outcome
        .result
        .expect("Git turn succeeds after the lock is released");
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

/// Closes issue #94's reopening gap: no composed runtime test previously
/// activated a real `ExistingGit` + `VaultGitMode::LocalHistory` Vault and
/// observed the subtree commit. Drives the *full* dispatch path — a real
/// `VaultWorkCoordinator`/`VaultWorkWorker` running production's
/// `dispatch_managed_git_turn`, which resolves to `run_local_history_git_turn`
/// — against a real `git2::Repository` whose root differs from the Vault
/// root, exactly like `dispatch_managed_git_turn_with_publishes_a_real_failure_through_the_full_async_path`
/// does for the managed-Git case above.
#[tokio::test]
async fn dispatch_managed_git_turn_commits_existing_git_local_history_drift_through_the_full_async_path()
 {
    let directory = tempdir().expect("temporary state directory");
    let repository_path = directory.path().join("repository");
    let repo = git2::Repository::init(&repository_path).expect("initialize repository");
    std::fs::write(repository_path.join("README.md"), "root readme").expect("root readme");
    {
        let mut index = repo.index().expect("index");
        index
            .add_path(Path::new("README.md"))
            .expect("stage readme");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");
        let signature =
            git2::Signature::now("Test", "test@example.test").expect("commit signature");
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "initial commit",
            &tree,
            &[],
        )
        .expect("initial commit");
    }
    let vault_subdirectory = repository_path.join("notes");
    std::fs::create_dir(&vault_subdirectory).expect("create Vault subdirectory");

    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let committed = registry
        .add(
            empty.revision(),
            NewVaultDefinition {
                name: "Local history".to_string(),
                enabled: true,
                source: RegistryVaultSource::ExistingGit {
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
        .expect("add local-history Vault");
    let vault_id = vault_id_named(&committed, "Local history");
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &committed);
    let control_block = collection.runtime(vault_id).expect("active runtime");

    // Drift existing before the Git turn runs: an uncommitted file inside
    // the Vault subdirectory.
    std::fs::write(vault_subdirectory.join("Idea.md"), "# idea\n").expect("write drift file");
    // Manual work directly in the repository root, outside the Vault
    // subdirectory: must never be staged or touched (containment).
    std::fs::write(repository_path.join("outside.md"), "manual outside work")
        .expect("write outside file");

    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
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
        .expect("Git turn dequeued");
    outcome.result.expect("local-history commit turn succeeds");

    // A new commit now exists containing exactly the Vault-subtree file.
    let repo = git2::Repository::open(&repository_path).expect("reopen repository");
    let head_commit = repo
        .head()
        .expect("HEAD")
        .peel_to_commit()
        .expect("HEAD commit");
    assert_eq!(head_commit.parent_count(), 1, "exactly one new commit made");
    let tree = head_commit.tree().expect("HEAD tree");
    assert!(
        tree.get_path(Path::new("notes/Idea.md")).is_ok(),
        "the Vault-subtree drift was committed"
    );
    assert!(
        tree.get_path(Path::new("outside.md")).is_err(),
        "work outside the Vault must never be staged or committed"
    );
    assert_eq!(
        std::fs::read_to_string(repository_path.join("outside.md"))
            .expect("outside file survives on disk"),
        "manual outside work",
        "manual local work must never be discarded or force-checked-out over"
    );

    let after = control_block.snapshot();
    assert_eq!(after.git, VaultGitStatus::Ready);
    assert!(after.git_error.is_none());
    assert_eq!(after.local_content, LocalContentStatus::ReadWrite);
    assert!(after.capabilities.browse);
    assert!(after.capabilities.mutate);
    assert!(
        !after.capabilities.pull && !after.capabilities.push,
        "Local history must never expose remote capabilities"
    );

    // A successful turn queues an Index turn, exactly like the managed-Git
    // path.
    let index_turn = worker
        .run_next(|request| async move {
            assert_eq!(request.vault_id(), vault_id);
            assert_eq!(request.kind(), VaultWorkKind::Index);
            Ok::<(), VaultWorkError>(())
        })
        .await
        .expect("successful local-history turn queues Index work");
    index_turn.result.expect("Index turn can proceed");
}

fn commit_file(repository: &git2::Repository, path: &str, contents: &str, message: &str) {
    let workdir = repository.workdir().expect("workdir");
    std::fs::write(workdir.join(path), contents).expect("write file");
    let mut index = repository.index().expect("index");
    index.add_path(Path::new(path)).expect("stage file");
    index.write().expect("write index");
    let tree = repository
        .find_tree(index.write_tree().expect("write tree"))
        .expect("find tree");
    let signature = git2::Signature::now("Test", "test@example.test").expect("signature");
    let parent = repository
        .head()
        .ok()
        .and_then(|head| head.peel_to_commit().ok());
    let parents = parent.iter().collect::<Vec<_>>();
    repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .expect("commit");
}

/// Build a local bare-repository fixture for an `ExistingGit` `PullOnly`/
/// `TwoWay` Vault: a source repository with one commit under `vault/`,
/// pushed to a bare "remote", and a `checkout` clone of that remote — the
/// `repository_path` an `ExistingGit` Vault's registry entry points at,
/// distinct from any Hatchdoor-managed clone. Mirrors `managed_sync.rs`'s
/// own `fixture` helper. Returns `(repository_path, remote_path)`; reused by
/// both the defect-1 composed dispatch test and the defect-2 `ExistingGit`
/// lock-contention test below, per the reopening's Spec review finding that
/// the two should share fixture-building rather than duplicate it.
fn existing_git_checkout_fixture(directory: &Path) -> (PathBuf, PathBuf) {
    let source_path = directory.join("source");
    let source = git2::Repository::init(&source_path).expect("source repository");
    std::fs::create_dir(source_path.join("vault")).expect("vault directory");
    commit_file(&source, "vault/Home.md", "# Home\n", "initial");
    let remote_path = directory.join("remote.git");
    git2::Repository::init_bare(&remote_path).expect("bare remote");
    source
        .find_remote("origin")
        .or_else(|_| source.remote("origin", remote_path.to_str().expect("remote path")))
        .expect("origin")
        .push(&["refs/heads/master:refs/heads/master"], None)
        .expect("initial push");

    let repository_path = directory.join("checkout");
    git2::Repository::clone(remote_path.to_str().expect("remote path"), &repository_path)
        .expect("existing checkout");
    (repository_path, remote_path)
}

/// Register an `ExistingGit` Vault in `mode` against `repository_path`,
/// activate it, and return its collection/registry/control-block/ID —
/// shared registration plumbing for the defect-1 and defect-2 `ExistingGit`
/// composed tests below, mirroring `managed_git_control_block`'s role for
/// the `ManagedGit` path.
fn existing_git_control_block(
    directory: &Path,
    name: &str,
    repository_path: PathBuf,
    mode: VaultGitMode,
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
                name: name.to_string(),
                enabled: true,
                source: RegistryVaultSource::ExistingGit {
                    repository_path,
                    // Registry-level validation requires a syntactically
                    // valid `https://` URL for `PullOnly`/`TwoWay`
                    // (`vault_registry.rs::normalize_https_repository_url`
                    // has no test-local-path allowance), but the real sync
                    // only ever reads the checkout's actual `origin` remote
                    // — never this field — so an unreachable placeholder is
                    // fine here.
                    repository_url: Some("https://example.test/vault.git".to_string()),
                    // Deliberately unconfigured: proves the fallback to the
                    // checkout's currently-checked-out branch.
                    branch: None,
                    vault_subdirectory: Some(PathBuf::from("vault")),
                    mode,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
            },
        )
        .expect("add ExistingGit Vault");
    let vault_id = vault_id_named(&committed, name);
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &committed);
    let control_block = collection.runtime(vault_id).expect("active runtime");
    (collection, registry, control_block, vault_id)
}

/// Closes issue #96's reopening defect 1: `dispatch_managed_git_turn_with`
/// used to return `Ok(())` immediately for every `ExistingGit` source in
/// `PullOnly`/`TwoWay` mode, so a real Pull-only or Two-way `ExistingGit`
/// Vault never actually synced with its remote. Drives a real `PullOnly`
/// `ExistingGit` Vault through the full async dispatch path — registry,
/// `VaultCollectionRuntime`, `VaultWorkCoordinator`/`VaultWorkWorker`,
/// `dispatch_managed_git_turn` — against a local bare-repository fixture
/// (the same `cfg!(test)` local-path allowance
/// `managed_sync.rs`'s own tests rely on), the same pattern as #94's
/// `dispatch_managed_git_turn_commits_existing_git_local_history_drift_through_the_full_async_path`.
///
/// Also exercises this ticket's open branch-resolution design decision: the
/// registry's `branch` is deliberately left `None`, proving the turn falls
/// back to whatever branch is currently checked out at `repository_path`
/// (`master`, from `git2::Repository::init`'s default) rather than failing
/// or guessing a different one.
///
/// Before defect 1's fix this failed: the remote commit made after the
/// checkout was created would never be fetched, since the turn was a no-op.
#[tokio::test]
async fn dispatch_managed_git_turn_synchronizes_existing_git_pull_only_through_the_full_async_path()
{
    let directory = tempdir().expect("temporary state directory");
    let (repository_path, remote_path) = existing_git_checkout_fixture(directory.path());

    // Someone else pushes a new commit to the remote before the turn runs.
    let actor_path = directory.path().join("actor");
    let actor = git2::Repository::clone(remote_path.to_str().expect("remote path"), &actor_path)
        .expect("actor checkout");
    commit_file(&actor, "vault/Remote.md", "remote note\n", "remote change");
    actor
        .find_remote("origin")
        .expect("origin")
        .push(&["refs/heads/master:refs/heads/master"], None)
        .expect("actor push");

    let (collection, registry, control_block, vault_id) = existing_git_control_block(
        directory.path(),
        "Existing pull-only",
        repository_path.clone(),
        VaultGitMode::PullOnly,
    );

    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
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
        .expect("Git turn dequeued");
    outcome.result.expect("pull-only sync succeeds");

    // The remote commit actually landed in the existing checkout — before
    // the fix this dispatch arm was a no-op and it never would have.
    assert_eq!(
        std::fs::read_to_string(repository_path.join("vault/Remote.md"))
            .expect("remote commit was pulled into the existing checkout"),
        "remote note\n"
    );

    let after = control_block.snapshot();
    assert_eq!(after.git, VaultGitStatus::Ready);
    assert!(after.git_error.is_none());
    assert!(after.capabilities.pull);
    assert!(
        !after.capabilities.mutate,
        "pull-only must never allow local mutation"
    );

    let index_turn = worker
        .run_next(|request| async move {
            assert_eq!(request.vault_id(), vault_id);
            assert_eq!(request.kind(), VaultWorkKind::Index);
            Ok::<(), VaultWorkError>(())
        })
        .await
        .expect("successful pull-only turn queues Index work");
    index_turn.result.expect("Index turn can proceed");
}

/// Closes issue #96's reopening defect 2 for the `ExistingGit` path
/// specifically (Spec review finding on this ticket's second round): the
/// `a_managed_git_turn_waits_for_a_concurrent_foreground_mutation_to_release_the_lock`
/// test above proves `acquire_mutation()` blocks a Git turn at the
/// `ManagedGit` call site, but the `ExistingGit` `PullOnly`/`TwoWay` arm
/// added for defect 1 has its own, separate `acquire_mutation()` call
/// site — same lock, same pattern, but not the same code, and this campaign
/// already hit a case (issue #95) where a "structurally identical" pair of
/// call sites diverged in a way code-review-by-inspection alone missed.
///
/// Proves the `ExistingGit` call site the same way, reusing
/// `existing_git_checkout_fixture`/`existing_git_control_block` (the same
/// fixture-building code `dispatch_managed_git_turn_synchronizes_existing_git_pull_only_through_the_full_async_path`
/// above uses, per that finding's request not to invent a new one):
/// acquires the mutation lock directly (simulating a foreground write), then
/// drives a real Pull-only `ExistingGit` turn through `dispatch_managed_git_turn`
/// (a real local sync against the bare-repository fixture — no injected
/// executor exists for this arm, unlike the `ManagedGit` test above), and
/// asserts it cannot complete while the lock is held and proceeds once it is
/// released.
///
/// Before defect 2's fix this failed the same way the `ManagedGit` test
/// above did: the turn raced straight through the 200ms window instead of
/// blocking, because the `ExistingGit` arm never acquired the lock at all.
#[tokio::test]
async fn an_existing_git_pull_only_turn_waits_for_a_concurrent_foreground_mutation_to_release_the_lock()
 {
    let directory = tempdir().expect("temporary state directory");
    let (repository_path, _remote_path) = existing_git_checkout_fixture(directory.path());
    let (collection, registry, control_block, vault_id) = existing_git_control_block(
        directory.path(),
        "Existing pull-only lock",
        repository_path,
        VaultGitMode::PullOnly,
    );

    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    coordinator.request(vault_id, VaultWorkKind::Git);

    // Simulate a foreground Markdown write already in flight, holding
    // exactly the lock a real write handler acquires.
    let mutation_guard = control_block
        .acquire_mutation()
        .await
        .expect("foreground mutation lock");

    let dispatch = worker.run_next(|request| {
        dispatch_managed_git_turn(
            &collection,
            &registry,
            &coordinator,
            &managed_git,
            "Hatchdoor",
            "hatchdoor@example.test",
            request,
        )
    });
    tokio::pin!(dispatch);

    let raced = tokio::time::timeout(std::time::Duration::from_millis(200), &mut dispatch).await;
    assert!(
        raced.is_err(),
        "the ExistingGit Git turn must block on the foreground mutation lock, not race past it"
    );

    drop(mutation_guard);

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), dispatch)
        .await
        .expect("Git turn proceeds once the foreground mutation releases the lock")
        .expect("Git turn dequeued");
    outcome
        .result
        .expect("Git turn succeeds after the lock is released");
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
