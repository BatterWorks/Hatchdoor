//! Turn-execution tests: one Index turn or one Git turn driven through the
//! same seam `server.rs`'s dispatch loop uses.

use super::*;
use std::path::PathBuf;
use tempfile::tempdir;

use crate::cache::SqliteCache;
use crate::cache::vault_snapshots::{VaultSnapshotFreshness, VaultSnapshotStatus};
use crate::embed::{Embedder, StubEmbedder};
use crate::runtime_config::RuntimeConfig;
use crate::search::vault_scoped::{VaultSearchCore, VaultSearchRequest};
use crate::search::{LayerSelection, NoteFilters, SearchMode};
use crate::vault_read::VaultScope;
use crate::vault_registry::{
    DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS, NewVaultDefinition, VaultRegistrySnapshot,
};
use crate::vault_work::ScheduleResult;

struct BlockingEmbedder {
    inner: StubEmbedder,
    entered: Arc<std::sync::Barrier>,
    release: Arc<std::sync::Barrier>,
}

struct ProbeEmbedder {
    inner: StubEmbedder,
    entered: std::sync::mpsc::Sender<()>,
}

impl Embedder for ProbeEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        self.entered
            .send(())
            .expect("mutation-boundary test is waiting for the scan probe");
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
    let both = add_local_vault(&registry, &one, "Second", second_path.clone());
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

    // Edit the note so the next turn genuinely has embedding work to do: a
    // rebuild of an *unchanged* Vault reuses its published vectors and never
    // calls the embedder, so `PanicEmbedder` would never fire and this would
    // assert nothing.
    std::fs::write(
        second_path.join("Home.md"),
        "# Home\n\nsecond version, edited",
    )
    .expect("edit the second Vault's note");

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
            searchable: true,
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

/// Regression: activation queues Index work before first-run model setup has
/// installed the embedder. The turn used to run anyway, wiping the cache
/// (placeholder identity vs. the stored one) and then panicking in the
/// chunker's tokenizer, so every restart paid a full reindex. It must defer
/// with a retryable error and leave the cache untouched instead.
#[tokio::test]
async fn index_turn_defers_while_the_embedding_model_is_still_being_set_up() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("create Vault directory");
    std::fs::write(vault_path.join("Note.md"), "# Note\n\nbody").expect("write note");

    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let snapshot = add_local_vault(&registry, &empty, "Only", vault_path);
    let vault_id = vault_id_named(&snapshot, "Only");
    let collection = VaultCollectionRuntime::new();
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &snapshot, &coordinator, &managed_git)
        .await;

    let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));
    cache
        .set_metadata("embedder_id", "stub-384")
        .expect("stamp the identity a previous build left behind");
    // An empty slot: exactly the state during model download/first-run setup.
    let embedder: Arc<dyn Embedder> = Arc::new(crate::embed::RuntimeEmbedder::new());

    coordinator.request(vault_id, VaultWorkKind::Index);
    let outcome = worker
        .run_next({
            let collection = collection.clone();
            let cache = cache.clone();
            let embedder = embedder.clone();
            move |request| async move {
                dispatch_vault_index_turn_with_embed_layers(
                    &collection,
                    cache,
                    embedder,
                    true,
                    request,
                )
                .await
            }
        })
        .await
        .expect("queued Index turn");

    let error = outcome
        .result
        .expect_err("the turn must defer rather than index against a missing model");
    assert_eq!(error.code(), "embedder_not_ready");
    assert!(
        error.retryable(),
        "the model-load path re-requests this work, so it must not be terminal"
    );
    assert_eq!(
        cache.get_metadata("embedder_id").expect("get").as_deref(),
        Some("stub-384"),
        "the deferred turn must leave the existing cache intact"
    );
}

/// The per-Vault Index dispatcher must carry the immutable embed-layer setting
/// into its candidate cache.  A demoted layer remains in the keyword read
/// model, while false explicitly suppresses its semantic vectors.
#[tokio::test]
async fn index_turn_with_embed_layers_disabled_keeps_demoted_notes_keyword_only() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(vault_path.join("sources")).expect("create Vault directory");
    std::fs::write(vault_path.join("sources/.hatchdoor-layer"), "sources")
        .expect("write layer marker");
    std::fs::write(
        vault_path.join("sources/Clip.md"),
        "# Clip\n\nmelatonin regulates the circadian rhythm",
    )
    .expect("write demoted note");

    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let snapshot = add_local_vault(&registry, &empty, "Only", vault_path);
    let vault_id = vault_id_named(&snapshot, "Only");
    let collection = VaultCollectionRuntime::new();
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &snapshot, &coordinator, &managed_git)
        .await;
    let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));

    let outcome = worker
        .run_next({
            let collection = collection.clone();
            let cache = cache.clone();
            let embedder = embedder.clone();
            move |request| async move {
                dispatch_vault_index_turn_with_embed_layers(
                    &collection,
                    cache,
                    embedder,
                    false,
                    request,
                )
                .await
            }
        })
        .await
        .expect("queued Index turn");
    outcome.result.expect("Index publication succeeds");

    let (layers, _) = LayerSelection::parse(&["sources".to_string()], &["sources".to_string()]);
    let search = VaultSearchCore::new(&cache, &collection, embedder.as_ref());
    let keyword = search
        .search(VaultSearchRequest {
            scope: VaultScope::One(vault_id),
            query: "melatonin".to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 1,
            filters: NoteFilters::default(),
            include_properties: Vec::new(),
            layers: layers.clone(),
        })
        .expect("keyword search");
    assert!(
        keyword
            .data
            .results
            .iter()
            .any(|hit| hit.note_slug == "clip"),
        "the demoted note remains keyword-searchable"
    );
    let semantic = search
        .search(VaultSearchRequest {
            scope: VaultScope::One(vault_id),
            query: "melatonin circadian".to_string(),
            mode: SearchMode::Semantic,
            limit: 10,
            per_note_cap: 1,
            filters: NoteFilters::default(),
            include_properties: Vec::new(),
            layers,
        })
        .expect("semantic search");
    assert!(
        semantic.data.results.is_empty(),
        "the disabled embed-layer setting must suppress demoted semantic vectors"
    );
}

/// An Index turn shares the foreground HTTP/MCP mutation boundary. Holding the
/// guard across a multi-file mutation must prevent the turn from scanning or
/// publishing a mixed snapshot; once the mutation completes, it publishes the
/// complete two-file state.
#[tokio::test]
async fn index_turn_waits_for_a_multifile_foreground_mutation_before_publishing() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("create Vault directory");
    let first_path = vault_path.join("First.md");
    let second_path = vault_path.join("Second.md");
    std::fs::write(&first_path, "# First\n\nbefore first").expect("write first note");
    std::fs::write(&second_path, "# Second\n\nbefore second").expect("write second note");

    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let snapshot = add_local_vault(&registry, &empty, "Only", vault_path);
    let vault_id = vault_id_named(&snapshot, "Only");
    let collection = VaultCollectionRuntime::new();
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    collection
        .reconcile_and_reconstruct(&registry, &snapshot, &coordinator, &managed_git)
        .await;
    let control = collection.runtime(vault_id).expect("active Vault runtime");
    let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));
    let (scan_entered, scan_probe) = std::sync::mpsc::channel();
    let embedder: Arc<dyn Embedder> = Arc::new(ProbeEmbedder {
        inner: StubEmbedder::new(384),
        entered: scan_entered,
    });

    // This is the same control-block guard acquired by HTTP and MCP write
    // adapters. Apply the two related file changes while it remains held.
    let mutation_guard = control
        .acquire_mutation()
        .await
        .expect("foreground mutation acquires its Vault lock");
    std::fs::write(&first_path, "# First\n\nafter first").expect("write first mutation");
    coordinator.request(vault_id, VaultWorkKind::Index);

    let mutation_probe = IndexMutationProbe::install(vault_id);
    let dispatch = tokio::spawn({
        let collection = collection.clone();
        let cache = cache.clone();
        let embedder = embedder.clone();
        async move {
            worker
                .run_next(move |request| {
                    let collection = collection.clone();
                    let cache = cache.clone();
                    let embedder = embedder.clone();
                    async move {
                        dispatch_vault_index_turn_with_embed_layers(
                            &collection,
                            cache,
                            embedder,
                            true,
                            request,
                        )
                        .await
                    }
                })
                .await
        }
    });
    mutation_probe.lock_attempted().await;
    assert!(
        matches!(
            scan_probe.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ),
        "after Index reaches its mutation-lock attempt, it must remain blocked before scanning"
    );
    assert_eq!(
        cache
            .snapshot_status(vault_id)
            .expect("read snapshot status"),
        None,
        "Index must not publish while the foreground mutation guard remains held"
    );
    assert_ne!(
        control.snapshot().search,
        VaultSearchStatus::Indexing,
        "Index must not advance runtime status before it acquires the foreground mutation guard"
    );

    std::fs::write(&second_path, "# Second\n\nafter second").expect("write second mutation");
    drop(mutation_guard);
    let outcome = dispatch
        .await
        .expect("worker task")
        .expect("Index turn ran");
    outcome.result.expect("Index publication succeeds");
    scan_probe
        .try_recv()
        .expect("scan begins after the foreground mutation releases");

    assert_eq!(
        cache
            .snapshot_note_content(vault_id, "first")
            .expect("read first snapshot")
            .as_deref(),
        Some("# First\n\nafter first")
    );
    assert_eq!(
        cache
            .snapshot_note_content(vault_id, "second")
            .expect("read second snapshot")
            .as_deref(),
        Some("# Second\n\nafter second")
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
            searchable: true,
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
                searchable: true,
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
            searchable: true,
        }),
        "a successful rebuild republishes fresh"
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
                archive_folder: None,
                commit_identity: None,
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
/// scheduler recording — via `dispatch_git_turn_with`'s injected
/// executor, rather than calling `publish_managed_git_turn_outcome`
/// directly. This is the "not just the generic coordinator mechanism"
/// coverage a real remote failure would exercise, without a reachable
/// remote or a network call in the test suite.
#[tokio::test]
async fn dispatch_git_turn_with_publishes_a_real_failure_through_the_full_async_path() {
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
            dispatch_git_turn_with(
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
            dispatch_git_turn_with(
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

/// Closes issue #96's reopening defect 2: `dispatch_git_turn_with`
/// used to run its blocking `git2` turn without ever acquiring
/// `VaultControlBlock::mutation_lock`, so a foreground Markdown write (which
/// acquires that same lock — see `handlers::vault_write::acquire_mutation`
/// and `mcp::tools::write::acquire_mutation`) could race a Git turn's
/// fetch/integrate/reset phases.
///
/// Proves the fix by acquiring the mutation lock directly in the test —
/// simulating a foreground write already in flight — then driving a real
/// managed-Git turn (via `dispatch_git_turn_with`'s injected
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
        dispatch_git_turn_with(
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
async fn dispatch_git_turn_is_a_no_op_for_a_non_managed_git_vault() {
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
    // directly here exercises `dispatch_git_turn`'s defensive
    // non-managed-Git branch without any real Git I/O.
    coordinator.request(vault_id, VaultWorkKind::Git);

    let outcome = worker
        .run_next(|request| {
            dispatch_git_turn(
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
/// `dispatch_git_turn`, which resolves to `run_local_history_git_turn`
/// — against a real `git2::Repository` whose root differs from the Vault
/// root, exactly like `dispatch_git_turn_with_publishes_a_real_failure_through_the_full_async_path`
/// does for the managed-Git case above.
#[tokio::test]
async fn dispatch_git_turn_commits_existing_git_local_history_drift_through_the_full_async_path() {
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
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
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
            dispatch_git_turn(
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
                    poll_interval_secs: DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS,
                },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add ExistingGit Vault");
    let vault_id = vault_id_named(&committed, name);
    let collection = VaultCollectionRuntime::new();
    collection.reconcile(&registry, &committed);
    let control_block = collection.runtime(vault_id).expect("active runtime");
    (collection, registry, control_block, vault_id)
}

/// Closes issue #96's reopening defect 1: `dispatch_git_turn_with`
/// used to return `Ok(())` immediately for every `ExistingGit` source in
/// `PullOnly`/`TwoWay` mode, so a real Pull-only or Two-way `ExistingGit`
/// Vault never actually synced with its remote. Drives a real `PullOnly`
/// `ExistingGit` Vault through the full async dispatch path — registry,
/// `VaultCollectionRuntime`, `VaultWorkCoordinator`/`VaultWorkWorker`,
/// `dispatch_git_turn` — against a local bare-repository fixture
/// (the same `cfg!(test)` local-path allowance
/// `managed_sync.rs`'s own tests rely on), the same pattern as #94's
/// `dispatch_git_turn_commits_existing_git_local_history_drift_through_the_full_async_path`.
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
async fn dispatch_git_turn_synchronizes_existing_git_pull_only_through_the_full_async_path() {
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
            dispatch_git_turn(
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
/// fixture-building code `dispatch_git_turn_synchronizes_existing_git_pull_only_through_the_full_async_path`
/// above uses, per that finding's request not to invent a new one):
/// acquires the mutation lock directly (simulating a foreground write), then
/// drives a real Pull-only `ExistingGit` turn through `dispatch_git_turn`
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
        dispatch_git_turn(
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

/// The executor reads the author defaults from the snapshot bound to each
/// turn rather than from a value captured once at startup, so saving a new
/// name or email applies to the next Git turn of every Vault without its own
/// commit identity — with no restart.
#[test]
fn git_author_defaults_follow_a_saved_settings_change_without_a_restart() {
    let runtime_config = RuntimeConfig::for_tests();

    assert_eq!(
        git_author_defaults(&runtime_config.snapshot()),
        ("Hatchdoor".to_string(), "hatchdoor@localhost".to_string()),
        "an unconfigured instance falls back to the documented defaults"
    );

    runtime_config
        .save([
            (
                "HATCHDOOR_GIT_AUTHOR_NAME".to_string(),
                "Second Author".to_string(),
            ),
            (
                "HATCHDOOR_GIT_AUTHOR_EMAIL".to_string(),
                "second@example.test".to_string(),
            ),
        ])
        .expect("save author defaults");

    // A turn dispatched after the save binds a fresh snapshot, exactly as
    // `VaultWorkExecutor::run` does.
    assert_eq!(
        git_author_defaults(&runtime_config.snapshot()),
        (
            "Second Author".to_string(),
            "second@example.test".to_string()
        )
    );
}

#[test]
fn startup_readiness_follows_collection_index_completion() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("create Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let snapshot = registry
        .add(
            0,
            NewVaultDefinition {
                name: "Startup Vault".to_string(),
                enabled: true,
                source: RegistryVaultSource::Local { path: vault_path },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add Vault");
    let vault_id = snapshot
        .definitions()
        .next()
        .expect("Vault definition")
        .vault_id();
    let vaults = VaultCollectionRuntime::new();
    vaults.reconcile(&registry, &snapshot);

    assert!(!collection_indexes_ready(&vaults));
    vaults
        .runtime(vault_id)
        .expect("active Vault")
        .set_search_status(VaultSearchStatus::Ready, None)
        .expect("publish ready search status");
    assert!(collection_indexes_ready(&vaults));
}

/// The executor binds the settings snapshot at the *start of each turn*, not
/// once when it is constructed: a save between two turns reaches the second
/// one, and the turn already running keeps the view it started with.
#[tokio::test]
async fn each_index_turn_binds_the_settings_snapshot_at_its_own_start() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(vault_path.join("sources")).expect("create Vault directory");
    std::fs::write(vault_path.join("sources/.hatchdoor-layer"), "sources")
        .expect("write layer marker");
    std::fs::write(
        vault_path.join("sources/Clip.md"),
        "# Clip\n\nmelatonin regulates the circadian rhythm",
    )
    .expect("write demoted note");

    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let snapshot = registry
        .add(
            0,
            NewVaultDefinition {
                name: "Only".to_string(),
                enabled: true,
                source: RegistryVaultSource::Local { path: vault_path },
                exclude_patterns: Vec::new(),
                https_credentials: None,
                archive_folder: None,
                commit_identity: None,
            },
        )
        .expect("add Vault");
    let vault_id = vault_id_named(&snapshot, "Only");
    let vaults = VaultCollectionRuntime::new();
    let (work, mut worker) = VaultWorkCoordinator::new();
    let managed_git = Arc::new(ManagedGitScheduler::new(work.clone()));
    vaults
        .reconcile_and_reconstruct(&registry, &snapshot, &work, &managed_git)
        .await;
    let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
    let runtime_config = RuntimeConfig::for_tests();
    runtime_config
        .save([("HATCHDOOR_EMBED_LAYERS".to_string(), "false".to_string())])
        .expect("save disabled setting");

    let executor = VaultWorkExecutor {
        vaults: vaults.clone(),
        registry: registry.clone(),
        work: work.clone(),
        managed_git: managed_git.clone(),
        cache: cache.clone(),
        embedder: embedder.clone(),
        runtime_config: runtime_config.clone(),
        startup: StartupTracker::scanning(),
        model_setup_started: Arc::new(AtomicBool::new(false)),
    };

    let outcome = worker
        .run_next(|request| executor.run(request))
        .await
        .expect("queued Index turn");
    outcome.result.expect("Index publication succeeds");

    let (layers, _) = LayerSelection::parse(&["sources".to_string()], &["sources".to_string()]);
    let search = VaultSearchCore::new(&cache, &vaults, embedder.as_ref());
    let semantic = |layers: LayerSelection| {
        search
            .search(VaultSearchRequest {
                scope: VaultScope::One(vault_id),
                query: "melatonin circadian".to_string(),
                mode: SearchMode::Semantic,
                limit: 10,
                per_note_cap: 1,
                filters: NoteFilters::default(),
                include_properties: Vec::new(),
                layers,
            })
            .expect("semantic search")
            .data
            .results
    };
    let keyword = search
        .search(VaultSearchRequest {
            scope: VaultScope::One(vault_id),
            query: "melatonin".to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 1,
            filters: NoteFilters::default(),
            include_properties: Vec::new(),
            layers: layers.clone(),
        })
        .expect("keyword search");
    assert!(
        keyword
            .data
            .results
            .iter()
            .any(|hit| hit.note_slug == "clip")
    );
    assert!(
        semantic(layers.clone()).is_empty(),
        "the first turn bound HATCHDOOR_EMBED_LAYERS=false, so the demoted note has no vectors"
    );

    // The same executor, with no restart and no reconstruction: the next turn
    // binds its own snapshot and picks the saved value up.
    runtime_config
        .save([("HATCHDOOR_EMBED_LAYERS".to_string(), "true".to_string())])
        .expect("save later setting");
    assert_eq!(
        work.request(vault_id, VaultWorkKind::Index),
        ScheduleResult::Queued
    );
    worker
        .run_next(|request| executor.run(request))
        .await
        .expect("second Index turn")
        .result
        .expect("second Index publication succeeds");
    assert!(
        !semantic(layers).is_empty(),
        "the second turn must observe the setting saved after the first one finished"
    );
}

/// AC2 of #197 at the executor seam: a turn driven through the work
/// coordinator lands its per-Vault status and index revision, and the
/// collection's own readiness conclusion follows from `publish_outcome` —
/// the rule that used to be inlined in `server.rs`'s loop.
#[tokio::test]
async fn publish_outcome_moves_startup_readiness_with_the_collections_index_turns() {
    let directory = tempdir().expect("temporary state directory");
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    std::fs::create_dir_all(&first_path).expect("first Vault directory");
    std::fs::create_dir_all(&second_path).expect("second Vault directory");
    std::fs::write(first_path.join("One.md"), "# One\n\nfirst note").expect("write first note");
    std::fs::write(second_path.join("Two.md"), "# Two\n\nsecond note").expect("write second note");

    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let with_first = add_local_vault(&registry, &empty, "First", first_path);
    let committed = add_local_vault(&registry, &with_first, "Second", second_path);
    let first = vault_id_named(&committed, "First");
    let second = vault_id_named(&committed, "Second");

    let vaults = VaultCollectionRuntime::new();
    let (work, mut worker) = VaultWorkCoordinator::new();
    let managed_git = Arc::new(ManagedGitScheduler::new(work.clone()));
    vaults
        .reconcile_and_reconstruct(&registry, &committed, &work, &managed_git)
        .await;
    let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
    let model_setup_started = Arc::new(AtomicBool::new(true));
    let executor = VaultWorkExecutor {
        vaults: vaults.clone(),
        registry: registry.clone(),
        work: work.clone(),
        managed_git: managed_git.clone(),
        cache: cache.clone(),
        embedder,
        runtime_config: RuntimeConfig::for_tests(),
        startup: StartupTracker::scanning(),
        model_setup_started: model_setup_started.clone(),
    };

    let drive = async |worker: &mut crate::vault_work::VaultWorkWorker| {
        let outcome = worker
            .run_next(|request| executor.run(request))
            .await
            .expect("reconstructed Index turn");
        executor.publish_outcome(&outcome);
        outcome
    };

    // Reconstruction queued one Index turn per active Vault. After the first,
    // the collection is not yet ready: the second Vault has never indexed.
    let first_turn = drive(&mut worker).await;
    let first_indexed = first_turn.request.vault_id();
    assert!(first_indexed == first || first_indexed == second);
    first_turn.result.expect("first Index turn succeeds");
    assert_eq!(
        vaults
            .runtime(first_indexed)
            .expect("indexed Vault runtime")
            .snapshot()
            .search,
        VaultSearchStatus::Ready
    );
    assert!(
        !executor.startup.is_ready(),
        "one indexed Vault out of two must not make the collection ready"
    );
    assert!(
        model_setup_started.load(Ordering::Acquire),
        "the model-setup flag stays set until the collection settles"
    );

    let second_turn = drive(&mut worker).await;
    assert_ne!(
        second_turn.request.vault_id(),
        first_indexed,
        "reconstruction queues one Index turn per active Vault"
    );
    second_turn.result.expect("second Index turn succeeds");
    assert!(
        executor.startup.is_ready(),
        "startup becomes ready once every active Vault's Index turn settled Ready"
    );
    assert!(
        !model_setup_started.load(Ordering::Acquire),
        "a settled collection clears the model-setup flag"
    );

    // A deferral while the embedder is still installing is explicitly exempt
    // from the failure branch: it must not knock startup out of readiness.
    executor.publish_outcome(&VaultWorkOutcome {
        request: first_turn.request,
        result: Err(VaultWorkError::new(
            "embedder_not_ready",
            "The search model is still being set up; indexing resumes when setup completes.",
            true,
        )),
    });
    assert!(
        executor.startup.is_ready(),
        "an embedder_not_ready deferral is not an indexing failure"
    );

    // Any other Index failure is, and it clears the model-setup flag too.
    model_setup_started.store(true, Ordering::Release);
    executor.publish_outcome(&VaultWorkOutcome {
        request: first_turn.request,
        result: Err(VaultWorkError::new(
            "vault_index_failed",
            "scan failed",
            true,
        )),
    });
    assert!(
        !executor.startup.is_ready(),
        "a real Index failure fails startup"
    );
    assert!(!model_setup_started.load(Ordering::Acquire));

    // A Git turn's outcome never moves startup readiness. Take a real Git
    // request from the coordinator rather than fabricating one — a `Local`
    // Vault's Git turn is a no-op, which is all this needs it for.
    assert_eq!(
        work.request(first_indexed, VaultWorkKind::Git),
        ScheduleResult::Queued
    );
    let git_turn = drive(&mut worker).await;
    assert_eq!(git_turn.request.kind(), VaultWorkKind::Git);
    git_turn
        .result
        .expect("a Local Vault's Git turn is a no-op");
    executor.startup.set_ready();
    executor.publish_outcome(&VaultWorkOutcome {
        request: git_turn.request,
        result: Err(VaultWorkError::new(
            "managed_git_unreachable",
            "no remote",
            true,
        )),
    });
    assert!(
        executor.startup.is_ready(),
        "readiness is an Index-turn conclusion only"
    );
}

/// Regression: a managed-Git Vault must keep polling on its own schedule,
/// turn after turn. The seams were each covered in isolation — the
/// scheduler's re-arm, the dispatch path's outcome publication — but not
/// the cycle they form, which is the only thing that makes a Vault poll
/// twice.
/// One full production cycle — the scheduler's tick requests the turn, the
/// dispatch path runs and publishes it, and the recorded outcome re-arms the
/// next attempt one poll interval out — driven through the same seams
/// `spawn_scheduler_tick` and the coordinator's worker loop use in
/// production.
#[tokio::test]
async fn a_managed_git_vault_keeps_polling_on_its_configured_interval() {
    let directory = tempdir().expect("temporary state directory");
    let (collection, registry, control_block, vault_id) =
        managed_git_control_block(directory.path());
    std::fs::create_dir_all(control_block.vault_path()).expect("already-acquired checkout");
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::new(coordinator.clone());
    let poll_interval = std::time::Duration::from_secs(3600);
    managed_git.activate(vault_id, poll_interval);

    // The first tick after activation must find the Vault due immediately.
    let started = std::time::Instant::now();
    managed_git.tick(started);
    let first = worker
        .run_next(|request| {
            dispatch_git_turn_with(
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
        .expect("the tick queued an initial Git turn");
    assert_eq!(first.request.kind(), VaultWorkKind::Git);
    first.result.expect("initial sync succeeds");
    // Drain the Index turn the successful Git turn queued.
    worker
        .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
        .await
        .expect("Index turn queued by the successful Git turn");

    // Nothing is due before the interval elapses.
    managed_git.tick(std::time::Instant::now());
    assert_eq!(
        coordinator.request(vault_id, VaultWorkKind::Git),
        ScheduleResult::Queued,
        "a Vault must not be re-requested before its interval elapses"
    );
    coordinator.drain_vault(vault_id);
    coordinator.activate_vault(vault_id);

    // Once the interval has elapsed, the tick must request the next turn.
    managed_git.tick(started + poll_interval + std::time::Duration::from_secs(1));
    let second = worker
        .run_next(|request| {
            dispatch_git_turn_with(
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
        .expect("the interval tick queued the next Git turn");
    assert_eq!(second.request.kind(), VaultWorkKind::Git);
    assert_eq!(second.request.vault_id(), vault_id);
    second.result.expect("scheduled re-sync succeeds");
}
