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
use crate::search::{LayerSelection, SearchMode};
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

/// [`BlockingEmbedder`] that parks on its first `embed` call only, for a
/// Vault with more than one note to embed.
struct OnceBlockingEmbedder {
    inner: BlockingEmbedder,
    parked: std::sync::atomic::AtomicBool,
}

impl Embedder for OnceBlockingEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if !self.parked.swap(true, std::sync::atomic::Ordering::SeqCst) {
            self.inner.entered.wait();
            self.inner.release.wait();
        }
        self.inner.inner.embed(texts)
    }

    fn embedding_dim(&self) -> usize {
        self.inner.inner.embedding_dim()
    }

    fn identity(&self) -> String {
        self.inner.inner.identity()
    }

    fn token_count(&self, text: &str, add_special_tokens: bool) -> Result<usize, String> {
        self.inner.inner.token_count(text, add_special_tokens)
    }
}

/// Parks a build inside its *read* phase. `token_count` is what the chunker
/// calls while note content is still being read and chunked, before any vector
/// work, so blocking the first call holds the turn exactly where its foreground
/// mutation guard must still be held. Only the first call parks; the rest of
/// the build runs normally.
struct ReadPhaseBlockingEmbedder {
    inner: StubEmbedder,
    entered: Arc<std::sync::Barrier>,
    release: Arc<std::sync::Barrier>,
    parked: std::sync::atomic::AtomicBool,
}

impl Embedder for ReadPhaseBlockingEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        self.inner.embed(texts)
    }

    fn embedding_dim(&self) -> usize {
        self.inner.embedding_dim()
    }

    fn identity(&self) -> String {
        self.inner.identity()
    }

    fn token_count(&self, text: &str, add_special_tokens: bool) -> Result<usize, String> {
        if !self.parked.swap(true, Ordering::SeqCst) {
            self.entered.wait();
            self.release.wait();
        }
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
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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
            VaultSearchStatus::Stale,
            "runtime status says what the retained generation answers during the turn"
        );
        assert!(
            collection
                .runtime(vault_id)
                .expect("active runtime")
                .snapshot()
                .capabilities
                .search,
            "the search capability holds through the embedding pass"
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
            .trees(
                crate::vault_read::VaultScope::One(vault_id),
                crate::vault_read::TreeScope::default(),
            )
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

/// One local Vault, activated and already indexed once, with a second Index
/// turn armed and something for it to embed.
///
/// The three tests below all need the same thing: a turn held open inside its
/// embedding pass, which is where a real Vault spends minutes and where issue
/// #223's write starvation lived.
struct EmbeddingTurnFixture {
    directory: tempfile::TempDir,
    vault_id: VaultId,
    collection: VaultCollectionRuntime,
    coordinator: VaultWorkCoordinator,
    worker: Option<crate::vault_work::VaultWorkWorker>,
    cache: Arc<SqliteCache>,
}

impl EmbeddingTurnFixture {
    async fn new() -> Self {
        let mut fixture = Self::unindexed().await;
        let mut worker = fixture.worker.take().expect("the fixture's worker");
        let working: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let published = worker
            .run_next({
                let collection = fixture.collection.clone();
                let cache = fixture.cache.clone();
                move |request| async move {
                    dispatch_vault_index_turn(&collection, cache, working, request).await
                }
            })
            .await
            .expect("initial Index turn");
        published.result.expect("initial publication succeeds");
        fixture.worker = Some(worker);

        // A write burst lands, which is what arms the next Index turn in
        // production: the watcher forwards the change intent to this
        // coordinator.
        std::fs::write(
            fixture.vault_path().join("Home.md"),
            "# Home\n\nmelatonin updated",
        )
        .expect("update note");
        fixture
            .coordinator
            .request(fixture.vault_id, VaultWorkKind::Index);
        fixture
    }

    /// The same Vault before its first Index turn, which activation has
    /// already armed: nothing published, nothing retained.
    async fn unindexed() -> Self {
        let directory = tempdir().expect("temporary state directory");
        let vault_path = directory.path().join("vault");
        std::fs::create_dir_all(&vault_path).expect("create Vault directory");
        std::fs::write(vault_path.join("Home.md"), "# Home\n\nmelatonin original")
            .expect("write note");

        let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let empty = match registry.load().expect("load empty registry") {
            crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
            crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
        };
        let snapshot = add_local_vault(&registry, &empty, "Only", vault_path.clone());
        let vault_id = vault_id_named(&snapshot, "Only");

        let collection = VaultCollectionRuntime::new();
        let (coordinator, worker) = VaultWorkCoordinator::new();
        let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
        collection
            .reconcile_and_reconstruct(&registry, &snapshot, &coordinator, &managed_git)
            .await;
        let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));

        Self {
            directory,
            vault_id,
            collection,
            coordinator,
            worker: Some(worker),
            cache,
        }
    }

    fn vault_path(&self) -> PathBuf {
        self.directory.path().join("vault")
    }

    fn control(&self) -> VaultControlBlock {
        self.collection
            .runtime(self.vault_id)
            .expect("active runtime")
    }

    fn snapshot_status(&self) -> Option<VaultSnapshotStatus> {
        self.cache
            .snapshot_status(self.vault_id)
            .expect("read snapshot status")
    }

    /// Run the armed Index turn with an embedder that parks inside `embed`,
    /// where a real Vault spends minutes and where the guard is now released.
    /// Returns the barrier reporting it has parked, the barrier that lets it
    /// go, and the turn itself.
    fn hold_open_mid_embedding(
        &mut self,
    ) -> (
        Arc<std::sync::Barrier>,
        Arc<std::sync::Barrier>,
        tokio::task::JoinHandle<Option<VaultWorkOutcome>>,
    ) {
        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let embedder: Arc<dyn Embedder> = Arc::new(BlockingEmbedder {
            inner: StubEmbedder::new(384),
            entered: entered.clone(),
            release: release.clone(),
        });
        (entered, release, self.run_armed_turn(embedder))
    }

    /// The same, parked one phase earlier: inside the read loop that chunks
    /// note content, where the guard must still be held.
    fn hold_open_mid_read_phase(
        &mut self,
    ) -> (
        Arc<std::sync::Barrier>,
        Arc<std::sync::Barrier>,
        tokio::task::JoinHandle<Option<VaultWorkOutcome>>,
    ) {
        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let embedder: Arc<dyn Embedder> = Arc::new(ReadPhaseBlockingEmbedder {
            inner: StubEmbedder::new(384),
            entered: entered.clone(),
            release: release.clone(),
            parked: std::sync::atomic::AtomicBool::new(false),
        });
        (entered, release, self.run_armed_turn(embedder))
    }

    fn run_armed_turn(
        &mut self,
        embedder: Arc<dyn Embedder>,
    ) -> tokio::task::JoinHandle<Option<VaultWorkOutcome>> {
        let mut worker = self.worker.take().expect("the fixture runs one held turn");
        let collection = self.collection.clone();
        let cache = self.cache.clone();
        tokio::spawn(async move {
            worker
                .run_next(move |request| {
                    let collection = collection.clone();
                    let cache = cache.clone();
                    async move {
                        dispatch_vault_index_turn(&collection, cache, embedder, request).await
                    }
                })
                .await
        })
    }
}

/// Meet one of the embedding pass's barriers from a blocking-pool thread. The
/// pass runs under `spawn_blocking`, so meeting its barrier from the test's
/// async context would park the runtime instead of the thread.
async fn meet_barrier(barrier: &Arc<std::sync::Barrier>) {
    let barrier = barrier.clone();
    tokio::task::spawn_blocking(move || barrier.wait())
        .await
        .expect("meet the embedding barrier");
}

/// Finish a held-open turn: release the parked blocking-pool thread, then take
/// its result. Always release before asserting — a panic with the barrier
/// still unmet hangs the runtime on drop instead of reporting the failure
/// (see `active_index_turn_reports_the_retained_snapshot_stale_to_concurrent_reads`).
async fn finish_held_turn(
    release: &Arc<std::sync::Barrier>,
    turn: tokio::task::JoinHandle<Option<VaultWorkOutcome>>,
) {
    meet_barrier(release).await;
    turn.await
        .expect("Index turn task")
        .expect("Index turn ran")
        .result
        .expect("Index turn publishes successfully");
}

/// Issue #483. An Index turn on a Vault that already answers search used to
/// report `Indexing` for its whole length, which withdraws the search
/// capability, while search went on answering from the retained generation.
/// The status now says what that generation supports. Held open in the read
/// phase, the earliest point a turn can be observed, and the status already
/// agrees with the snapshot.
#[tokio::test]
async fn an_index_turn_on_a_searchable_vault_reports_stale_and_keeps_the_search_capability() {
    let mut fixture = EmbeddingTurnFixture::new().await;
    let control = fixture.control();
    assert_eq!(control.snapshot().search, VaultSearchStatus::Ready);
    let (entered, release, turn) = fixture.hold_open_mid_read_phase();
    meet_barrier(&entered).await;

    let during = control.snapshot();
    let snapshot_during = fixture.snapshot_status();

    finish_held_turn(&release, turn).await;

    assert_eq!(
        during.search,
        VaultSearchStatus::Stale,
        "a rebuild of a searchable Vault reports what its retained generation answers"
    );
    assert!(
        during.capabilities.search,
        "a Vault that keeps answering search keeps the search capability"
    );
    assert_eq!(
        during.search_error, None,
        "a running rebuild is not a failure"
    );
    assert_eq!(
        snapshot_during.map(|status| status.freshness),
        Some(VaultSnapshotFreshness::Stale),
        "the status and the snapshot's freshness agree while the turn runs"
    );
    assert_eq!(
        control.snapshot().search,
        VaultSearchStatus::Ready,
        "a turn that publishes cleanly ends ready"
    );
}

/// The other side of #483: a Vault with nothing retained has nothing to
/// answer from, so its first turn still reports `Indexing` and no search
/// capability.
#[tokio::test]
async fn a_first_index_turn_still_reports_indexing_without_the_search_capability() {
    let mut fixture = EmbeddingTurnFixture::unindexed().await;
    let control = fixture.control();
    let (entered, release, turn) = fixture.hold_open_mid_read_phase();
    meet_barrier(&entered).await;

    let during = control.snapshot();

    finish_held_turn(&release, turn).await;

    assert_eq!(during.search, VaultSearchStatus::Indexing);
    assert!(!during.capabilities.search);
    assert_eq!(control.snapshot().search, VaultSearchStatus::Ready);
}

/// A retained generation with no vectors participates but can only answer a
/// search with nothing, so a rebuild over it never grants the capability,
/// the rule `retained_search_status` applies to a paused or failed turn.
#[tokio::test]
async fn an_index_turn_over_a_vectorless_generation_never_grants_the_search_capability() {
    let mut fixture = EmbeddingTurnFixture::unindexed().await;
    let control = fixture.control();
    let index = control.authoritative_index().expect("scan the Vault");
    assert!(
        fixture
            .cache
            .publish_vault_structure_snapshot(
                fixture.vault_id,
                &index,
                &StubEmbedder::new(384),
                true,
            )
            .expect("publish the structure-only generation"),
        "the Vault retains a participating generation with no vectors"
    );

    let (read_entered, read_release, turn) = fixture.hold_open_mid_read_phase();
    meet_barrier(&read_entered).await;
    let during = control.snapshot();
    finish_held_turn(&read_release, turn).await;

    assert_eq!(during.search, VaultSearchStatus::Indexing);
    assert!(
        !during.capabilities.search,
        "a vectorless retained generation must not grant the search capability"
    );
}

/// The narrowing stops at the read phase. A foreground mutation arriving while
/// the turn is still reading and chunking note content waits, exactly as
/// before, which is what keeps a turn from ever observing half of a multi-file
/// write. The control afterwards proves the guard was merely held: the same
/// acquisition succeeds the moment the read phase ends.
#[tokio::test]
async fn a_foreground_mutation_waits_while_an_index_turn_is_still_reading() {
    let mut fixture = EmbeddingTurnFixture::new().await;
    let control = fixture.control();
    let (entered, release, turn) = fixture.hold_open_mid_read_phase();
    meet_barrier(&entered).await;

    let blocked = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        control.acquire_mutation(),
    )
    .await
    .is_err();

    finish_held_turn(&release, turn).await;

    assert!(
        blocked,
        "a foreground mutation must not start while an Index turn is still reading the Vault"
    );
    tokio::time::timeout(
        std::time::Duration::from_millis(500),
        control.acquire_mutation(),
    )
    .await
    .expect("the guard is free once the turn is over")
    .expect("mutation acquisition succeeds");
}

/// Issue #223, inverted. An Index turn used to hold this Vault's foreground
/// mutation lock for the whole turn, embedding included, and every HTTP and
/// MCP Markdown write takes that same lock first — `vault_mutation`'s
/// primitives, `mcp::tools::write::acquire_mutation`, and the `batch` tool,
/// which takes it once for a whole batch. `acquire_mutation` is an unbounded
/// await with no timeout and no backpressure, so on a CPU-only host a write
/// arriving during a seven-minute embedding pass did not degrade, it parked:
/// the caller's transport gave up on a write that then landed anyway, which
/// is an at-least-once hazard the moment the caller retries.
///
/// The guard now spans the read phase only, so the write starts while the turn
/// is still embedding.
#[tokio::test]
async fn a_foreground_mutation_acquires_the_lock_while_an_index_turn_embeds() {
    let mut fixture = EmbeddingTurnFixture::new().await;
    let control = fixture.control();
    let (entered, release, turn) = fixture.hold_open_mid_embedding();
    meet_barrier(&entered).await;

    // Parked inside `Embedder::embed`, exactly where a real Vault spends
    // minutes. This is the call every write makes first.
    let acquired = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        control.acquire_mutation(),
    )
    .await
    .is_ok();

    finish_held_turn(&release, turn).await;

    assert!(
        acquired,
        "a foreground mutation must start while an Index turn embeds, not wait out the pass"
    );
}

/// The other half of the decision: a write that lands mid-embedding makes the
/// generation about to be published already behind the Markdown, so it
/// publishes stale rather than claiming to be current. It keeps participating
/// and keeps answering search — the label is what changes, and the watcher has
/// already armed the catch-up turn.
#[tokio::test]
async fn a_mutation_during_the_embedding_pass_publishes_the_generation_stale() {
    let mut fixture = EmbeddingTurnFixture::new().await;
    let control = fixture.control();
    let (entered, release, turn) = fixture.hold_open_mid_embedding();
    meet_barrier(&entered).await;

    // One complete foreground mutation, taken and released exactly as an HTTP
    // or MCP Markdown write does.
    let guard = control
        .acquire_mutation()
        .await
        .expect("foreground mutation acquires its Vault lock");
    std::fs::write(
        fixture.vault_path().join("Later.md"),
        "# Later\n\nmelatonin arrived after the scan",
    )
    .expect("write the mid-pass mutation");
    drop(guard);

    finish_held_turn(&release, turn).await;

    assert_eq!(
        fixture.snapshot_status(),
        Some(VaultSnapshotStatus {
            participating: true,
            freshness: VaultSnapshotFreshness::Stale,
            searchable: true,
        }),
        "a generation built across a foreground mutation must not publish itself fresh"
    );
    assert_eq!(
        fixture.control().snapshot().search,
        VaultSearchStatus::Stale,
        "and the runtime says the same thing `retained_snapshot_search_status` would \
         derive from that row after a restart, rather than claiming Ready"
    );
    assert!(
        fixture.control().snapshot().capabilities.search,
        "a stale generation keeps the search capability; it is labelled, not withheld"
    );

    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
    let response = VaultSearchCore::new(&fixture.cache, &fixture.collection, embedder.as_ref())
        .search(VaultSearchRequest {
            scope: VaultScope::One(fixture.vault_id),
            query: "melatonin".to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 1,
            layers: LayerSelection::default_surface(),
        })
        .expect("keyword search against the stale generation");
    assert!(
        response
            .data
            .results
            .iter()
            .any(|hit| hit.note_slug == "home"),
        "a stale generation still answers search"
    );
    assert!(
        response.partial,
        "and reports itself a non-fresh participant while doing so"
    );
}

/// The control for the test above: the same released-and-retaken guard, with
/// nothing intervening. A turn that nothing wrote under still publishes fresh.
#[tokio::test]
async fn an_index_turn_with_no_concurrent_mutation_publishes_fresh() {
    let mut fixture = EmbeddingTurnFixture::new().await;
    let (entered, release, turn) = fixture.hold_open_mid_embedding();
    meet_barrier(&entered).await;

    assert_eq!(
        fixture.snapshot_status().map(|status| status.freshness),
        Some(VaultSnapshotFreshness::Stale),
        "precondition: the retained generation reads stale for the length of the rebuild"
    );

    finish_held_turn(&release, turn).await;

    assert_eq!(
        fixture.snapshot_status(),
        Some(VaultSnapshotStatus {
            participating: true,
            freshness: VaultSnapshotFreshness::Fresh,
            searchable: true,
        }),
        "releasing the guard across the embedding pass must not make every turn publish stale"
    );
    assert_eq!(
        fixture.control().snapshot().search,
        VaultSearchStatus::Ready,
        "and the runtime still settles Ready, which is what startup readiness latches on"
    );
}

/// Issue #329: a snapshot attempt that starts while an Index turn is still
/// building supersedes that turn, and the turn's publication then writes
/// nothing. The turn used to count that as a publication and mark its Vault
/// `Ready` on the strength of rows it never wrote. It now fails retryably,
/// leaves the row exactly as the newer attempt left it, and does not mark it
/// stale over that attempt's verdict.
#[tokio::test]
async fn a_superseded_index_turn_fails_retryably_and_leaves_the_row_to_the_newer_attempt() {
    let mut fixture = EmbeddingTurnFixture::new().await;
    let (entered, release, turn) = fixture.hold_open_mid_embedding();
    meet_barrier(&entered).await;

    // `mark_vault_snapshot_stale` begins its own snapshot attempt, which is
    // exactly the concurrent caller the attempt guard exists for. The newer
    // attempt then decides the row's freshness; setting it `fresh` here makes
    // any stale mark the superseded turn wrongly applied afterwards visible.
    fixture
        .cache
        .mark_vault_snapshot_stale(fixture.vault_id)
        .expect("a newer attempt supersedes the running turn");
    fixture
        .cache
        .connection()
        .expect("open the shared cache")
        .execute(
            "UPDATE vault_snapshots SET freshness = 'fresh' WHERE vault_id = ?1",
            [fixture.vault_id.to_string()],
        )
        .expect("the newer attempt settles the row fresh");

    meet_barrier(&release).await;
    let outcome = turn
        .await
        .expect("Index turn task")
        .expect("Index turn ran");
    let error = outcome
        .result
        .expect_err("a superseded turn published nothing and must not report success");
    assert_eq!(error.code(), "vault_index_failed");
    assert!(error.retryable(), "a superseded turn is worth retrying");

    assert_eq!(
        fixture.snapshot_status(),
        Some(VaultSnapshotStatus {
            participating: true,
            freshness: VaultSnapshotFreshness::Fresh,
            searchable: true,
        }),
        "the superseded turn must not stale the row the newer attempt owns"
    );
    assert_eq!(
        fixture
            .cache
            .snapshot_note_content(fixture.vault_id, "home")
            .expect("read the retained snapshot")
            .as_deref(),
        Some("# Home\n\nmelatonin original"),
        "the superseded turn's candidate was never published"
    );
    let runtime = fixture.control().snapshot();
    assert_ne!(
        runtime.search,
        VaultSearchStatus::Ready,
        "a turn that published nothing must not report its Vault current"
    );
    assert_eq!(
        runtime
            .search_error
            .as_ref()
            .map(|error| error.code.as_str()),
        Some("vault_index_failed")
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
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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

/// #323: a successful commit turn used to republish `Ready`, erasing a
/// sync conflict and its file list within one save of it appearing. A
/// conflict survives commit turns; a sync that succeeds clears it.
#[test]
fn a_sync_conflict_survives_successful_commit_turns_until_a_sync_resolves_it() {
    let directory = tempdir().expect("temporary state directory");
    let (_collection, _registry, control_block, vault_id) =
        managed_git_control_block(directory.path());
    std::fs::create_dir_all(control_block.vault_path()).expect("acquired checkout root");
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
    managed_git.activate(
        vault_id,
        std::time::Duration::from_secs(DEFAULT_MANAGED_GIT_POLL_INTERVAL_SECS),
    );
    let cooldown = crate::git::CommitCooldown::new();
    publish_managed_git_turn_outcome(
        &control_block,
        &coordinator,
        &managed_git,
        vault_id,
        &Err(VaultWorkError::new(
            "managed_git_conflict",
            "managed checkout merge conflict: vault/Home.md",
            false,
        )
        .with_detail(crate::vault_work::VaultWorkErrorDetail::AffectedPaths(
            vec!["vault/Home.md".to_string()],
        ))),
    );
    let published = control_block.snapshot();

    for _ in 0..2 {
        finish_commit_turn(
            &control_block,
            &cooldown,
            vault_id,
            Ok(crate::git::ManagedGitOutcome::Synchronized),
        )
        .expect("commit turn succeeded");
    }

    let after = control_block.snapshot();
    assert_eq!(after.git, VaultGitStatus::Unavailable);
    assert_eq!(after.git_error, published.git_error);
    assert!(
        after
            .git_error
            .as_ref()
            .is_some_and(|error| error.detail.is_some()),
        "the conflicted file list must survive too"
    );

    publish_managed_git_turn_outcome(
        &control_block,
        &coordinator,
        &managed_git,
        vault_id,
        &Ok(crate::git::ManagedGitOutcome::Synchronized),
    );
    let resolved = control_block.snapshot();
    assert_eq!(resolved.git, VaultGitStatus::Ready);
    assert!(resolved.git_error.is_none());
}

/// The other half of #323's rule: a failure a commit turn can itself
/// produce is cleared by the next commit turn that succeeds.
#[test]
fn a_successful_commit_turn_clears_a_failure_a_commit_could_have_caused() {
    let directory = tempdir().expect("temporary state directory");
    let (_collection, _registry, control_block, vault_id) =
        managed_git_control_block(directory.path());
    std::fs::create_dir_all(control_block.vault_path()).expect("acquired checkout root");
    let cooldown = crate::git::CommitCooldown::new();
    let _ = finish_commit_turn(
        &control_block,
        &cooldown,
        vault_id,
        Err(VaultWorkError::new(
            "managed_git_dirty_working_copy",
            "managed checkout has unsupported local work: outside.txt",
            false,
        )),
    );
    assert_eq!(control_block.snapshot().git, VaultGitStatus::Unavailable);

    finish_commit_turn(
        &control_block,
        &cooldown,
        vault_id,
        Ok(crate::git::ManagedGitOutcome::Synchronized),
    )
    .expect("commit turn succeeded");

    let after = control_block.snapshot();
    assert_eq!(after.git, VaultGitStatus::Ready);
    assert!(after.git_error.is_none());
}

#[test]
fn publish_managed_git_turn_outcome_isolates_a_failure_from_already_acquired_local_markdown() {
    let directory = tempdir().expect("temporary state directory");
    let (_collection, _registry, control_block, vault_id) =
        managed_git_control_block(directory.path());
    std::fs::create_dir_all(control_block.vault_path()).expect("acquired checkout root");
    let (coordinator, _worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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
                |_config, _lease, _ledger| Ok(crate::git::ManagedGitOutcome::UpToDate),
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
                |_config, _lease, _ledger| {
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
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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
            |_config, _lease, _ledger| Ok(crate::git::ManagedGitOutcome::UpToDate),
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
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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

/// ADR-31 decision 4, through the real turns: the coordinator admits a
/// Vault's commit and sync beside its own Index turn, and the Vault's
/// mutation lock is what decides how long they wait. They wait out the read
/// phase, when the Index turn is reading the notes they would rewrite, and
/// never the embedding pass, which is where a large Vault spends hours.
#[tokio::test]
async fn a_vaults_git_work_waits_for_its_index_read_phase_but_not_its_embedding() {
    for park_in_read_phase in [true, false] {
        let directory = tempdir().expect("temporary state directory");
        let (repository_path, _remote_path) = existing_git_checkout_fixture(directory.path());
        // Two-way, so the commit turn takes the mutation lock as well as the
        // sync does.
        let (collection, registry, _control_block, vault_id) = existing_git_control_block(
            directory.path(),
            "Existing two-way lanes",
            repository_path,
            VaultGitMode::TwoWay,
        );
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
        let cooldown = crate::git::CommitCooldown::new();
        let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));

        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let embedder: Arc<dyn Embedder> = if park_in_read_phase {
            Arc::new(ReadPhaseBlockingEmbedder {
                inner: StubEmbedder::new(384),
                entered: entered.clone(),
                release: release.clone(),
                parked: std::sync::atomic::AtomicBool::new(false),
            })
        } else {
            Arc::new(BlockingEmbedder {
                inner: StubEmbedder::new(384),
                entered: entered.clone(),
                release: release.clone(),
            })
        };

        coordinator.request(vault_id, VaultWorkKind::Index);
        coordinator.request(vault_id, VaultWorkKind::Commit);
        coordinator.request(vault_id, VaultWorkKind::Git);
        let index_turn = worker.next_turn().await.expect("Index turn admitted");
        assert_eq!(index_turn.request().kind(), VaultWorkKind::Index);
        let index = tokio::spawn({
            let collection = collection.clone();
            let cache = cache.clone();
            async move {
                index_turn
                    .run(|request| dispatch_vault_index_turn(&collection, cache, embedder, request))
                    .await
            }
        });
        meet_barrier(&entered).await;

        let (collection, registry, managed_git, cooldown, coordinator) = (
            &collection,
            &registry,
            &managed_git,
            &cooldown,
            &coordinator,
        );
        for (position, kind) in [VaultWorkKind::Commit, VaultWorkKind::Git]
            .into_iter()
            .enumerate()
        {
            let turn = tokio::time::timeout(std::time::Duration::from_secs(1), worker.next_turn())
                .await
                .expect("the coordinator admits the Vault's Git work beside its Index turn")
                .expect("Git-lane turn admitted");
            assert_eq!(turn.request().kind(), kind);
            let outcome = {
                let git_work = turn.run(|request| async move {
                    if kind == VaultWorkKind::Commit {
                        dispatch_commit_turn(
                            collection,
                            registry,
                            managed_git,
                            cooldown,
                            "Hatchdoor",
                            "hatchdoor@example.test",
                            request,
                        )
                        .await
                    } else {
                        dispatch_git_turn(
                            collection,
                            registry,
                            coordinator,
                            managed_git,
                            "Hatchdoor",
                            "hatchdoor@example.test",
                            request,
                        )
                        .await
                    }
                });
                tokio::pin!(git_work);

                let raced =
                    tokio::time::timeout(std::time::Duration::from_millis(300), &mut git_work)
                        .await;
                if park_in_read_phase && position == 0 {
                    let held = raced.is_err();
                    meet_barrier(&release).await;
                    assert!(
                        held,
                        "a commit must not touch notes the Index turn is reading"
                    );
                    tokio::time::timeout(std::time::Duration::from_secs(5), &mut git_work)
                        .await
                        .expect("the commit proceeds once the read phase ends")
                } else {
                    let finished = raced.is_ok();
                    if !finished && !park_in_read_phase {
                        meet_barrier(&release).await;
                    }
                    assert!(
                        finished,
                        "{kind:?} work must not wait out its own Vault's embedding pass"
                    );
                    raced.expect("finished")
                }
            };
            outcome
                .result
                .unwrap_or_else(|error| panic!("{kind:?} turn fails: {error:?}"));
            drop(turn);
        }
        if !park_in_read_phase {
            assert!(
                !index.is_finished(),
                "both turns finished while the Index turn was still embedding"
            );
            meet_barrier(&release).await;
        }
        index.await.expect("Index turn task");
    }
}

/// A Two-way `ExistingGit` Vault whose first Index turn has published, with
/// one Git-lane turn having run inside that turn's embedding pass.
struct GitWorkInsideAnEmbeddingPass {
    _directory: tempfile::TempDir,
    vault_id: VaultId,
    collection: VaultCollectionRuntime,
    control_block: VaultControlBlock,
    worker: crate::vault_work::VaultWorkWorker,
    cache: Arc<SqliteCache>,
    git_result: Result<(), VaultWorkError>,
}

impl GitWorkInsideAnEmbeddingPass {
    /// `prepare` gets the checkout and its bare remote before the Index turn
    /// reads anything, so whatever it leaves on disk is in that turn's scan.
    async fn run(kind: VaultWorkKind, prepare: impl FnOnce(&Path, &Path)) -> Self {
        let directory = tempdir().expect("temporary state directory");
        let (repository_path, remote_path) = existing_git_checkout_fixture(directory.path());
        prepare(&repository_path, &remote_path);
        let (collection, registry, control_block, vault_id) = existing_git_control_block(
            directory.path(),
            "Git inside embedding",
            repository_path,
            VaultGitMode::TwoWay,
        );
        let (coordinator, mut worker) = VaultWorkCoordinator::new();
        let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
        let cooldown = crate::git::CommitCooldown::new();
        let cache = Arc::new(SqliteCache::in_memory(384).expect("open shared cache"));

        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let embedder: Arc<dyn Embedder> = Arc::new(OnceBlockingEmbedder {
            inner: BlockingEmbedder {
                inner: StubEmbedder::new(384),
                entered: entered.clone(),
                release: release.clone(),
            },
            parked: std::sync::atomic::AtomicBool::new(false),
        });
        coordinator.request(vault_id, VaultWorkKind::Index);
        let index_turn = worker.next_turn().await.expect("Index turn admitted");
        let index = tokio::spawn({
            let collection = collection.clone();
            let cache = cache.clone();
            async move {
                index_turn
                    .run(|request| dispatch_vault_index_turn(&collection, cache, embedder, request))
                    .await
            }
        });
        meet_barrier(&entered).await;

        coordinator.request(vault_id, kind);
        let git_turn = worker.next_turn().await.expect("Git-lane turn admitted");
        assert_eq!(git_turn.request().kind(), kind);
        let git_result = {
            let (collection, registry, managed_git, cooldown, coordinator) = (
                &collection,
                &registry,
                &managed_git,
                &cooldown,
                &coordinator,
            );
            git_turn
                .run(|request| async move {
                    if kind == VaultWorkKind::Commit {
                        dispatch_commit_turn(
                            collection,
                            registry,
                            managed_git,
                            cooldown,
                            "Hatchdoor",
                            "hatchdoor@example.test",
                            request,
                        )
                        .await
                    } else {
                        dispatch_git_turn(
                            collection,
                            registry,
                            coordinator,
                            managed_git,
                            "Hatchdoor",
                            "hatchdoor@example.test",
                            request,
                        )
                        .await
                    }
                })
                .await
                .result
        };

        // Released before anything is asserted: a panic with the barrier
        // unmet hangs the runtime on drop instead of reporting the failure.
        meet_barrier(&release).await;
        let index_result = index.await.expect("Index turn task").result;
        index_result.expect("Index turn publishes");

        Self {
            _directory: directory,
            vault_id,
            collection,
            control_block,
            worker,
            cache,
            git_result,
        }
    }

    /// Run the Index turn the Git work left queued, and fail if it left none.
    async fn run_queued_catch_up(&mut self) -> Arc<dyn Embedder> {
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let catch_up = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.worker.run_next({
                let collection = self.collection.clone();
                let cache = self.cache.clone();
                let embedder = embedder.clone();
                move |request| async move {
                    assert_eq!(request.kind(), VaultWorkKind::Index);
                    dispatch_vault_index_turn(&collection, cache, embedder, request).await
                }
            }),
        )
        .await
        .expect("the sync queued the catch-up Index turn itself")
        .expect("catch-up Index turn dequeued");
        catch_up.result.expect("catch-up Index turn publishes");
        embedder
    }

    fn snapshot_freshness(&self) -> Option<VaultSnapshotFreshness> {
        self.cache
            .snapshot_status(self.vault_id)
            .expect("read snapshot status")
            .map(|status| status.freshness)
    }
}

/// Issue #549. A commit turn holds the Vault's mutation guard like a write
/// does, and every holder used to count as a write. A commit turn that found
/// nothing to commit inside an Index turn's embedding pass therefore made
/// that turn publish stale, with no changed file for the watcher to answer
/// with a catch-up turn. The Vault then reported `stale` over a current index
/// until something unrelated reindexed it.
#[tokio::test]
async fn a_commit_turn_with_nothing_to_commit_leaves_a_concurrent_index_turn_fresh() {
    let overlap = GitWorkInsideAnEmbeddingPass::run(VaultWorkKind::Commit, |_, _| {}).await;
    overlap.git_result.as_ref().expect("commit turn succeeds");

    assert_eq!(
        overlap.snapshot_freshness(),
        Some(VaultSnapshotFreshness::Fresh),
        "a commit turn that changed no file must not make the Index turn publish stale"
    );
    assert_eq!(
        overlap.control_block.snapshot().search,
        VaultSearchStatus::Ready
    );
}

/// The same for a commit turn that does commit. The note was on disk before
/// the Index turn scanned, so the published generation already holds it:
/// committing moves `HEAD` and leaves the working tree as it was.
#[tokio::test]
async fn a_commit_turn_that_commits_leaves_a_concurrent_index_turn_fresh() {
    let overlap = GitWorkInsideAnEmbeddingPass::run(VaultWorkKind::Commit, |checkout, _| {
        std::fs::write(checkout.join("vault/Mine.md"), "# mine\n").expect("write note");
    })
    .await;
    overlap.git_result.as_ref().expect("commit turn succeeds");

    let checkout =
        git2::Repository::open(overlap.control_block.vault_path().parent().expect("root"))
            .expect("open checkout");
    let head = checkout
        .head()
        .expect("HEAD")
        .peel_to_commit()
        .expect("HEAD commit");
    assert!(
        head.tree()
            .expect("HEAD tree")
            .get_path(Path::new("vault/Mine.md"))
            .is_ok(),
        "precondition: the commit turn committed the note"
    );
    assert_eq!(
        overlap.snapshot_freshness(),
        Some(VaultSnapshotFreshness::Fresh),
        "a commit rewrites no Markdown, so the generation is not behind it"
    );
    assert_eq!(
        overlap.control_block.snapshot().search,
        VaultSearchStatus::Ready
    );
}

/// A sync that pulls a remote edit does rewrite Markdown, after the Index
/// turn read it. That generation is behind and says so, and the sync has
/// queued the Index turn that catches up, with no `refresh_vault`.
#[tokio::test]
async fn a_sync_that_pulls_markdown_inside_an_embedding_pass_is_followed_by_a_fresh_index() {
    let mut overlap = GitWorkInsideAnEmbeddingPass::run(VaultWorkKind::Git, |_, remote| {
        let actor_path = remote.parent().expect("fixture root").join("actor");
        let actor = git2::Repository::clone(remote.to_str().expect("remote path"), &actor_path)
            .expect("actor checkout");
        commit_file(
            &actor,
            "vault/Theirs.md",
            "# Theirs\n\nvalerian pulled from the remote\n",
            "their commit",
        );
        actor
            .find_remote("origin")
            .expect("origin")
            .push(&["refs/heads/master:refs/heads/master"], None)
            .expect("actor push");
    })
    .await;
    overlap.git_result.as_ref().expect("sync succeeds");

    assert_eq!(
        overlap.snapshot_freshness(),
        Some(VaultSnapshotFreshness::Stale),
        "the generation was read before the pull and must not claim to be current"
    );
    assert_eq!(
        overlap.control_block.snapshot().search,
        VaultSearchStatus::Stale
    );

    let embedder = overlap.run_queued_catch_up().await;

    assert_eq!(
        overlap.snapshot_freshness(),
        Some(VaultSnapshotFreshness::Fresh)
    );
    assert_eq!(
        overlap.control_block.snapshot().search,
        VaultSearchStatus::Ready
    );
    let response = VaultSearchCore::new(&overlap.cache, &overlap.collection, embedder.as_ref())
        .search(VaultSearchRequest {
            scope: VaultScope::One(overlap.vault_id),
            query: "valerian".to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 1,
            layers: LayerSelection::default_surface(),
        })
        .expect("keyword search against the caught-up generation");
    assert!(
        response
            .data
            .results
            .iter()
            .any(|hit| hit.note_slug == "theirs"),
        "the pulled note is in the published generation"
    );
}

/// A sync that pushes a local commit and pulls nothing rewrites no note, so
/// the Index turn it overlapped is not behind anything.
#[tokio::test]
async fn a_sync_that_only_pushes_leaves_a_concurrent_index_turn_fresh() {
    let overlap = GitWorkInsideAnEmbeddingPass::run(VaultWorkKind::Git, |checkout, _| {
        std::fs::write(checkout.join("vault/Mine.md"), "# mine\n").expect("write note");
    })
    .await;
    overlap.git_result.as_ref().expect("sync succeeds");

    assert_eq!(
        overlap.snapshot_freshness(),
        Some(VaultSnapshotFreshness::Fresh),
        "committing and pushing changes no file the Index turn read"
    );
    assert_eq!(
        overlap.control_block.snapshot().search,
        VaultSearchStatus::Ready
    );
}

/// A failed sync cannot say whether it rewrote a note: a merge can land
/// before its push is refused. The Index turn it overlapped publishes stale,
/// and because nothing else follows a failed sync with an Index turn, the
/// sync queues the catch-up itself. Here the remote is gone, so nothing was
/// rewritten and the catch-up settles the Vault `Ready`.
#[tokio::test]
async fn a_failed_sync_inside_an_embedding_pass_queues_its_own_catch_up_index_turn() {
    let mut overlap = GitWorkInsideAnEmbeddingPass::run(VaultWorkKind::Git, |_, remote| {
        std::fs::remove_dir_all(remote).expect("remove the remote");
    })
    .await;
    overlap
        .git_result
        .as_ref()
        .expect_err("precondition: the sync fails without its remote");

    assert_eq!(
        overlap.control_block.snapshot().search,
        VaultSearchStatus::Stale
    );
    overlap.run_queued_catch_up().await;

    assert_eq!(
        overlap.snapshot_freshness(),
        Some(VaultSnapshotFreshness::Fresh)
    );
    assert_eq!(
        overlap.control_block.snapshot().search,
        VaultSearchStatus::Ready
    );
}

/// With no Index turn admitted there is no verdict for a failed sync to
/// spoil, so it queues nothing: a Vault whose remote is down must not reindex
/// on every retry.
#[tokio::test]
async fn a_failed_sync_with_no_index_turn_admitted_queues_no_index_turn() {
    let directory = tempdir().expect("temporary state directory");
    let (repository_path, remote_path) = existing_git_checkout_fixture(directory.path());
    std::fs::remove_dir_all(remote_path).expect("remove the remote");
    let (collection, registry, _control_block, vault_id) = existing_git_control_block(
        directory.path(),
        "Remote down",
        repository_path,
        VaultGitMode::TwoWay,
    );
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
    coordinator.request(vault_id, VaultWorkKind::Git);

    worker
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
        .expect("Git turn dequeued")
        .result
        .expect_err("precondition: the sync fails without its remote");

    assert!(!coordinator.has_work(vault_id, VaultWorkKind::Index));
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

    assert!(!collection_indexes_settled(&vaults));
    let runtime = vaults.runtime(vault_id).expect("active Vault");
    runtime
        .set_search_status(VaultSearchStatus::Indexing, None)
        .expect("publish indexing search status");
    assert!(
        !collection_indexes_settled(&vaults),
        "a turn still running has not settled"
    );
    runtime
        .set_search_status(VaultSearchStatus::Ready, None)
        .expect("publish ready search status");
    assert!(collection_indexes_settled(&vaults));
    runtime
        .set_search_status(
            VaultSearchStatus::Unavailable,
            Some(VaultRuntimeError {
                code: "vault_index_failed".to_string(),
                message: "scan failed".to_string(),
                retryable: true,
                detail: None,
            }),
        )
        .expect("publish failed search status");
    assert!(
        collection_indexes_settled(&vaults),
        "a Vault whose turn failed has settled; the failure is its own status (#326)"
    );
}

/// A Vault with no directory has nothing to index, so it must not hold the
/// rest of the collection out of readiness (#326).
#[test]
fn a_vault_without_a_directory_does_not_hold_the_collection_unsettled() {
    let directory = tempdir().expect("temporary state directory");
    let present = directory.path().join("present");
    std::fs::create_dir_all(&present).expect("create Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let with_present = add_local_vault(&registry, &empty, "Present", present);
    let missing_path = directory.path().join("missing");
    std::fs::create_dir_all(&missing_path).expect("create Vault directory");
    let committed = add_local_vault(&registry, &with_present, "Missing", missing_path.clone());
    // Registered while it existed, gone by the time the runtime activates:
    // the dev fixture's missing-path Vault, and a moved or unmounted one.
    std::fs::remove_dir_all(&missing_path).expect("remove Vault directory");
    let vaults = VaultCollectionRuntime::new();
    vaults.reconcile(&registry, &committed);
    let missing = vaults
        .runtime(vault_id_named(&committed, "Missing"))
        .expect("a Vault without a directory is still an active runtime");
    assert_ne!(missing.snapshot().activation, VaultActivationStatus::Active);

    vaults
        .runtime(vault_id_named(&committed, "Present"))
        .expect("present Vault")
        .set_search_status(VaultSearchStatus::Ready, None)
        .expect("publish ready search status");
    assert!(collection_indexes_settled(&vaults));
}

fn loaded_registry(registry: &VaultRegistryStore) -> VaultRegistrySnapshot {
    match registry.load().expect("load registry") {
        VaultRegistryState::Ready(snapshot) => snapshot,
        VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    }
}

/// 2.8.0 starts with no Vaults (ADR-40), so no Index turn ever runs to settle
/// startup. Once the model is set up there is nothing left to wait for (#453).
#[test]
fn an_instance_with_no_vaults_is_ready_once_the_model_is_set_up() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let vaults = VaultCollectionRuntime::new();
    let startup = StartupTracker::scanning();
    let model_setup_started = AtomicBool::new(true);

    assert!(collection_indexes_settled(&vaults));
    settle_startup(&startup, &vaults, &registry, &model_setup_started);

    assert!(startup.collection_indexes_ready());
    assert_eq!(startup.status().state, "ready");
    assert!(
        !model_setup_started.load(Ordering::Acquire),
        "settling releases the model-setup claim, as it does after an Index turn"
    );
}

/// A Vault that is disabled is not active, so an instance whose Vaults are
/// all disabled waits on nothing either (#453).
#[test]
fn an_instance_whose_vaults_are_all_disabled_is_ready_once_the_model_is_set_up() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("create Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let added = add_local_vault(&registry, &loaded_registry(&registry), "Only", vault_path);
    let disabled = registry
        .disable(added.revision(), vault_id_named(&added, "Only"))
        .expect("disable the Vault");
    let vaults = VaultCollectionRuntime::new();
    vaults.reconcile(&registry, &disabled);
    let startup = StartupTracker::scanning();

    settle_startup(&startup, &vaults, &registry, &AtomicBool::new(true));

    assert!(startup.collection_indexes_ready());
}

/// A registry that needs operator recovery activates no Vault, but it is not
/// an empty instance: nothing can be served until it is recovered, so a
/// script waiting on `/ready` must keep waiting (#453).
#[test]
fn a_registry_awaiting_recovery_is_not_ready_although_no_vault_is_active() {
    let directory = tempdir().expect("temporary state directory");
    let registry_path = directory.path().join("state/vaults.json");
    std::fs::create_dir_all(registry_path.parent().expect("state directory"))
        .expect("create state directory");
    std::fs::write(&registry_path, "not a registry").expect("write a corrupt registry");
    let registry = VaultRegistryStore::new(registry_path);
    assert!(matches!(
        registry.load().expect("load registry"),
        VaultRegistryState::Recovery(_)
    ));
    let vaults = VaultCollectionRuntime::new();
    let startup = StartupTracker::scanning();
    let model_setup_started = AtomicBool::new(true);

    settle_startup(&startup, &vaults, &registry, &model_setup_started);

    assert!(!startup.collection_indexes_ready());
    assert_eq!(startup.status().state, "scanning");
    assert!(model_setup_started.load(Ordering::Acquire));
}

/// Model setup gates readiness whatever the Vault count: with terms
/// outstanding, a download in flight or a failed setup, an instance with no
/// Vaults is not ready. It becomes ready when the model has loaded (#453).
#[test]
fn an_instance_with_no_vaults_is_not_ready_while_model_setup_is_pending() {
    let directory = tempdir().expect("temporary state directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let vaults = VaultCollectionRuntime::new();
    let startup = StartupTracker::terms_required();
    let model_setup_started = AtomicBool::new(true);
    let settle = || settle_startup(&startup, &vaults, &registry, &model_setup_started);

    settle();
    assert_eq!(startup.status().state, "terms_required");

    startup.set_downloading("EmbeddingGemma 300M Q4", Some(1), Some(2));
    settle();
    assert_eq!(startup.status().state, "downloading");

    startup.set_model_setup_failed();
    settle();
    assert_eq!(startup.status().state, "failed");
    assert!(model_setup_started.load(Ordering::Acquire));

    // What a finished model setup does before it asks.
    startup.set_scanning();
    settle();
    assert_eq!(startup.status().state, "ready");
}

/// The last Vault still in its first index leaving the active set leaves
/// nothing to wait for. No Index turn reports that, so the collection's own
/// change does (#453).
#[tokio::test]
async fn disabling_the_only_vault_still_in_its_first_index_settles_startup() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("create Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let added = add_local_vault(&registry, &loaded_registry(&registry), "Only", vault_path);
    let vault_id = vault_id_named(&added, "Only");
    let vaults = VaultCollectionRuntime::new();
    vaults.reconcile(&registry, &added);
    vaults
        .runtime(vault_id)
        .expect("active Vault")
        .set_search_status(VaultSearchStatus::Indexing, None)
        .expect("publish indexing search status");
    let startup = StartupTracker::scanning();
    let model_setup_started = Arc::new(AtomicBool::new(true));
    // While the Vault is active and unsettled, asking settles nothing.
    settle_startup(&startup, &vaults, &registry, &model_setup_started);
    assert!(!startup.collection_indexes_ready());
    let watching = tokio::spawn(settle_startup_on_collection_changes(
        startup.clone(),
        vaults.clone(),
        registry.clone(),
        model_setup_started,
    ));

    let disabled = registry
        .disable(added.revision(), vault_id)
        .expect("disable the Vault");
    vaults.reconcile(&registry, &disabled);

    wait_until_ready(&startup).await;
    watching.abort();
}

/// Disconnecting it does the same: the Vault leaves the registry, and the
/// collection with it (#453).
#[tokio::test]
async fn disconnecting_the_only_vault_still_in_its_first_index_settles_startup() {
    let directory = tempdir().expect("temporary state directory");
    let vault_path = directory.path().join("vault");
    std::fs::create_dir_all(&vault_path).expect("create Vault directory");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let added = add_local_vault(&registry, &loaded_registry(&registry), "Only", vault_path);
    let vault_id = vault_id_named(&added, "Only");
    let vaults = VaultCollectionRuntime::new();
    vaults.reconcile(&registry, &added);
    let startup = StartupTracker::scanning();
    let watching = tokio::spawn(settle_startup_on_collection_changes(
        startup.clone(),
        vaults.clone(),
        registry.clone(),
        Arc::new(AtomicBool::new(true)),
    ));
    assert!(!startup.collection_indexes_ready());

    let disconnected = registry
        .disconnect(added.revision(), vault_id)
        .expect("disconnect the Vault");
    vaults.reconcile(&registry, &disconnected);

    wait_until_ready(&startup).await;
    watching.abort();
}

async fn wait_until_ready(startup: &StartupTracker) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !startup.collection_indexes_ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("startup settles once the collection has no Vault in its first index");
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
    let managed_git = Arc::new(ManagedGitScheduler::without_durable_state(work.clone()));
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
        commit_cooldown: Arc::new(crate::git::CommitCooldown::new()),
        cache: cache.clone(),
        embedder: embedder.clone(),
        runtime_config: runtime_config.clone(),
        startup: StartupTracker::scanning(),
        model_setup_started: Arc::new(AtomicBool::new(false)),
        index_retries: IndexRetries::default(),
        index_slice: INDEX_TURN_SLICE,
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
    let managed_git = Arc::new(ManagedGitScheduler::without_durable_state(work.clone()));
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
        commit_cooldown: Arc::new(crate::git::CommitCooldown::new()),
        cache: cache.clone(),
        embedder,
        runtime_config: RuntimeConfig::for_tests(),
        startup: StartupTracker::scanning(),
        model_setup_started: model_setup_started.clone(),
        index_retries: IndexRetries::default(),
        index_slice: INDEX_TURN_SLICE,
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
        !executor.startup.collection_indexes_ready(),
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
        executor.startup.collection_indexes_ready(),
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
        executor.startup.collection_indexes_ready(),
        "an embedder_not_ready deferral is not an indexing failure"
    );

    // Nor is a real Index failure of one Vault: it is that Vault's own
    // status, and the other Vault is still serving (#326).
    executor.publish_outcome(&VaultWorkOutcome {
        request: first_turn.request,
        result: Err(VaultWorkError::new(
            "vault_index_failed",
            "scan failed",
            true,
        )),
    });
    assert!(
        executor.startup.collection_indexes_ready(),
        "one Vault's Index failure does not take the instance out of readiness"
    );

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
        executor.startup.collection_indexes_ready(),
        "readiness is an Index-turn conclusion only"
    );
}

/// Regression: a managed-Git Vault must keep polling on its own schedule,
/// turn after turn. The seams were each covered in isolation — the
/// scheduler's re-arm, the dispatch path's outcome publication — but not the
/// cycle they form, which is the only thing that makes a Vault poll twice.
/// So this drives one full production cycle: the scheduler's tick requests
/// the turn, the dispatch path runs and publishes it, and the recorded
/// outcome re-arms the next attempt one poll interval out — through the same
/// seams `spawn_scheduler_tick` and the coordinator's worker loop use.
#[tokio::test]
async fn a_managed_git_vault_keeps_polling_on_its_configured_interval() {
    let directory = tempdir().expect("temporary state directory");
    let (collection, registry, control_block, vault_id) =
        managed_git_control_block(directory.path());
    std::fs::create_dir_all(control_block.vault_path()).expect("already-acquired checkout");
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
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
                |_config, _lease, _ledger| Ok(crate::git::ManagedGitOutcome::UpToDate),
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
                |_config, _lease, _ledger| Ok(crate::git::ManagedGitOutcome::UpToDate),
            )
        })
        .await
        .expect("the interval tick queued the next Git turn");
    assert_eq!(second.request.kind(), VaultWorkKind::Git);
    assert_eq!(second.request.vault_id(), vault_id);
    second.result.expect("scheduled re-sync succeeds");
}

/// Run exactly one queued commit turn through the same seam `server.rs`'s
/// dispatch loop uses, and return its result. Every commit-turn test below
/// needs this identical eight-line block, and none of them is about the
/// block.
async fn run_one_commit_turn(
    collection: &VaultCollectionRuntime,
    registry: &VaultRegistryStore,
    managed_git: &ManagedGitScheduler,
    cooldown: &crate::git::CommitCooldown,
    worker: &mut crate::vault_work::VaultWorkWorker,
) -> Result<(), VaultWorkError> {
    worker
        .run_next(|request| {
            dispatch_commit_turn(
                collection,
                registry,
                managed_git,
                cooldown,
                "Hatchdoor",
                "hatchdoor@example.test",
                request,
            )
        })
        .await
        .expect("commit turn dequeued")
        .result
}

/// The heart of #267 for a Vault that *does* have a remote: a Two-way Vault
/// used to commit only inside the turn that also fetched and pushed, and that
/// turn only ran on the Vault's sync schedule, a day by default. A note
/// written to such a Vault could sit uncommitted for 24 hours.
///
/// Drives a real commit turn through the full async path and asserts both
/// halves of the split: the local commit happened, and the remote was not
/// touched. The remote is proved untouched from both directions: a commit
/// someone else pushed before this turn is still not in the local checkout
/// afterwards (no fetch), and the remote's own branch has not moved (no push).
#[tokio::test]
async fn a_commit_turn_commits_a_two_way_vault_without_touching_its_remote() {
    let directory = tempdir().expect("temporary state directory");
    let (repository_path, remote_path) = existing_git_checkout_fixture(directory.path());

    // Someone else pushes to the remote before the turn runs. A sync turn
    // would integrate this; a commit turn must not even look.
    let actor_path = directory.path().join("actor");
    let actor = git2::Repository::clone(remote_path.to_str().expect("remote path"), &actor_path)
        .expect("actor checkout");
    commit_file(&actor, "vault/Theirs.md", "# theirs\n", "their commit");
    actor
        .find_remote("origin")
        .expect("origin")
        .push(&["refs/heads/master:refs/heads/master"], None)
        .expect("actor push");
    let remote_head_before = git2::Repository::open(&remote_path)
        .expect("open remote")
        .refname_to_id("refs/heads/master")
        .expect("remote master");

    let (collection, registry, control_block, vault_id) = existing_git_control_block(
        directory.path(),
        "Two way",
        repository_path.clone(),
        VaultGitMode::TwoWay,
    );

    // The write this turn exists to commit, plus the Vault's own local HEAD
    // before it runs.
    std::fs::write(repository_path.join("vault/Mine.md"), "# mine\n").expect("write note");
    let local_head_before = git2::Repository::open(&repository_path)
        .expect("open checkout")
        .head()
        .expect("HEAD")
        .peel_to_commit()
        .expect("HEAD commit")
        .id();

    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
    let cooldown = crate::git::CommitCooldown::new();
    coordinator.request(vault_id, VaultWorkKind::Commit);
    run_one_commit_turn(&collection, &registry, &managed_git, &cooldown, &mut worker)
        .await
        .expect("the commit turn succeeds");

    let checkout = git2::Repository::open(&repository_path).expect("reopen checkout");
    let head = checkout
        .head()
        .expect("HEAD")
        .peel_to_commit()
        .expect("HEAD commit");
    assert_ne!(head.id(), local_head_before, "a local commit was made");
    assert!(
        head.tree()
            .expect("HEAD tree")
            .get_path(Path::new("vault/Mine.md"))
            .is_ok(),
        "the write is in the commit this turn made"
    );
    assert!(
        head.tree()
            .expect("HEAD tree")
            .get_path(Path::new("vault/Theirs.md"))
            .is_err(),
        "the commit turn never fetched, so the remote's newer commit is not here"
    );
    assert_eq!(
        git2::Repository::open(&remote_path)
            .expect("open remote")
            .refname_to_id("refs/heads/master")
            .expect("remote master"),
        remote_head_before,
        "the commit turn never pushed, so the remote's branch has not moved"
    );

    let after = control_block.snapshot();
    assert_eq!(after.git, VaultGitStatus::Ready);
    assert!(after.git_error.is_none());

    // The watcher change that asked for this commit already asked for the
    // reindex; a commit turn must not queue a second one.
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(25),
            worker.run_next(|_| async { Ok::<(), VaultWorkError>(()) }),
        )
        .await
        .is_err(),
        "a commit turn queues no work of its own"
    );
}

/// Pull-only is out of scope by design: such a Vault refuses writes, so it
/// has no changes of its own to commit, and its turn must leave a folder its
/// operator dirtied by hand exactly as it found it.
#[tokio::test]
async fn a_commit_turn_is_a_no_op_for_a_pull_only_vault() {
    let directory = tempdir().expect("temporary state directory");
    let (repository_path, _remote_path) = existing_git_checkout_fixture(directory.path());
    let (collection, registry, control_block, vault_id) = existing_git_control_block(
        directory.path(),
        "Pull only",
        repository_path.clone(),
        VaultGitMode::PullOnly,
    );
    std::fs::write(repository_path.join("vault/Manual.md"), "# by hand\n").expect("write by hand");
    let head_before = git2::Repository::open(&repository_path)
        .expect("open checkout")
        .head()
        .expect("HEAD")
        .peel_to_commit()
        .expect("HEAD commit")
        .id();
    let git_before = control_block.snapshot().git;

    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
    let cooldown = crate::git::CommitCooldown::new();
    coordinator.request(vault_id, VaultWorkKind::Commit);
    run_one_commit_turn(&collection, &registry, &managed_git, &cooldown, &mut worker)
        .await
        .expect("a Vault that does not commit reports no failure");

    assert_eq!(
        git2::Repository::open(&repository_path)
            .expect("reopen checkout")
            .head()
            .expect("HEAD")
            .peel_to_commit()
            .expect("HEAD commit")
            .id(),
        head_before,
        "a Pull-only Vault commits nothing"
    );
    assert_eq!(
        std::fs::read_to_string(repository_path.join("vault/Manual.md")).expect("file survives"),
        "# by hand\n",
        "and leaves the operator's own file alone"
    );
    assert_eq!(
        control_block.snapshot().git,
        git_before,
        "its Git status is untouched"
    );
}

/// Committing frequently means failing frequently when the cause of the
/// failure is standing, as drift outside the Vault's own folder is, and only
/// the operator can clear it. The cooldown is what stops that becoming one
/// failed turn per save; a manual commit, and the fix landing, are what stop
/// the cooldown outliving the condition.
#[tokio::test]
async fn a_failed_commit_turn_reports_its_paths_and_suppresses_the_next_automatic_one() {
    let directory = tempdir().expect("temporary state directory");
    let (repository_path, _remote_path) = existing_git_checkout_fixture(directory.path());
    let (collection, registry, control_block, vault_id) = existing_git_control_block(
        directory.path(),
        "Two way",
        repository_path.clone(),
        VaultGitMode::TwoWay,
    );
    std::fs::write(repository_path.join("vault/Mine.md"), "# mine\n").expect("write note");
    std::fs::write(repository_path.join("outside.md"), "manual outside work")
        .expect("write outside file");

    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
    let cooldown = crate::git::CommitCooldown::with_period(std::time::Duration::from_secs(300));
    coordinator.request(vault_id, VaultWorkKind::Commit);
    let failure = run_one_commit_turn(&collection, &registry, &managed_git, &cooldown, &mut worker)
        .await
        .expect_err("drift outside the Vault fails the commit");
    assert_eq!(failure.code(), "managed_git_dirty_working_copy");

    let after = control_block.snapshot();
    assert_eq!(after.git, VaultGitStatus::Unavailable);
    let published = after
        .git_error
        .expect("the failure reaches the Vault status");
    assert_eq!(published.code, "managed_git_dirty_working_copy");
    assert_eq!(
        published.detail,
        Some(VaultRuntimeErrorDetail::AffectedPaths {
            paths: vec!["outside.md".to_string()],
            total: 1,
        }),
        "the console's affected-paths detail names what the operator has to fix"
    );

    for _ in 0..5 {
        assert!(
            !cooldown.try_admit(vault_id),
            "however many changes arrive, none starts another automatic commit"
        );
    }

    // The operator commits the outside drift. Nothing else intervenes: the
    // next automatic attempt after the cooldown has to succeed on its own.
    let checkout = git2::Repository::open(&repository_path).expect("reopen checkout");
    commit_file(&checkout, "outside.md", "manual outside work", "outside");
    assert_eq!(
        cooldown.due(std::time::Instant::now() + std::time::Duration::from_secs(301)),
        vec![vault_id],
        "the suppressed changes are owed exactly one turn once the window closes"
    );

    coordinator.request(vault_id, VaultWorkKind::Commit);
    run_one_commit_turn(&collection, &registry, &managed_git, &cooldown, &mut worker)
        .await
        .expect("the Vault resumes committing with no manual action");
    assert_eq!(control_block.snapshot().git, VaultGitStatus::Ready);
    assert!(
        cooldown.try_admit(vault_id),
        "a successful commit clears the suppression"
    );
    assert!(
        git2::Repository::open(&repository_path)
            .expect("reopen checkout")
            .head()
            .expect("HEAD")
            .peel_to_commit()
            .expect("HEAD commit")
            .tree()
            .expect("HEAD tree")
            .get_path(Path::new("vault/Mine.md"))
            .is_ok(),
        "and the note that was waiting is committed"
    );
}

/// A commit is not a check of the remote, so it must not move the schedule
/// that governs one. Without this a Vault would push a day later than its
/// interval says every time a note was written just before its turn was due.
#[tokio::test]
async fn a_commit_turn_leaves_the_remote_sync_schedule_where_it_was() {
    let directory = tempdir().expect("temporary state directory");
    let (repository_path, _remote_path) = existing_git_checkout_fixture(directory.path());
    let (collection, registry, _control_block, vault_id) = existing_git_control_block(
        directory.path(),
        "Two way",
        repository_path.clone(),
        VaultGitMode::TwoWay,
    );
    std::fs::write(repository_path.join("vault/Mine.md"), "# mine\n").expect("write note");

    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
    managed_git.activate(vault_id, std::time::Duration::from_secs(3600));
    // Arm a schedule the way a completed sync turn would, so there is a
    // deadline for the commit below to be proved not to have moved.
    managed_git.record_outcome(vault_id, &Ok(crate::git::ManagedGitOutcome::UpToDate));
    let armed = managed_git
        .next_attempt_for_test(vault_id)
        .expect("a tracked Vault has an armed attempt");

    let cooldown = crate::git::CommitCooldown::new();
    coordinator.request(vault_id, VaultWorkKind::Commit);
    run_one_commit_turn(&collection, &registry, &managed_git, &cooldown, &mut worker)
        .await
        .expect("the commit turn succeeds");

    assert_eq!(
        managed_git.next_attempt_for_test(vault_id),
        Some(armed),
        "a commit turn is not a remote check and never re-arms the sync schedule"
    );
    assert_eq!(
        managed_git.poll_interval_for_test(vault_id),
        Some(std::time::Duration::from_secs(3600)),
        "nor does it change the interval"
    );
}

/// #323: a Git turn that cannot read the registry publishes this failure as
/// the Vault's `git_error`, which every client sees. The registry error's own
/// text names the registry file's absolute host path.
#[test]
fn an_unreadable_registry_is_reported_without_the_host_path() {
    let directory = tempdir().expect("temporary directory");
    let registry_path = directory.path().join("state/vaults.json");
    std::fs::create_dir_all(&registry_path).expect("a directory where the file should be");
    let registry = VaultRegistryStore::new(registry_path);
    let vault_id = VaultId::generate().expect("vault id");

    let error = git_credentials(&registry, vault_id).expect_err("registry cannot be read");

    assert_eq!(error.code(), "managed_git_registry_unavailable");
    assert!(error.retryable());
    assert!(
        !error.message().contains('/'),
        "client-visible message leaks a host path: {}",
        error.message()
    );
}

/// Two Local Vaults reconstructed into a collection, with an executor over
/// them that starts where a fresh process does: model installed, tracker
/// scanning, nothing indexed yet.
async fn two_vault_executor(
    directory: &Path,
) -> (
    VaultWorkExecutor,
    crate::vault_work::VaultWorkWorker,
    VaultId,
    VaultId,
    PathBuf,
) {
    let first_path = directory.join("first");
    let second_path = directory.join("second");
    std::fs::create_dir_all(&first_path).expect("first Vault directory");
    std::fs::create_dir_all(&second_path).expect("second Vault directory");
    std::fs::write(first_path.join("One.md"), "# One\n\nfirst note").expect("write first note");
    std::fs::write(second_path.join("Two.md"), "# Two\n\nsecond note").expect("write second note");
    let registry = VaultRegistryStore::new(directory.join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let with_first = add_local_vault(&registry, &empty, "First", first_path);
    let committed = add_local_vault(&registry, &with_first, "Second", second_path.clone());
    let first = vault_id_named(&committed, "First");
    let second = vault_id_named(&committed, "Second");
    let vaults = VaultCollectionRuntime::new();
    let (work, worker) = VaultWorkCoordinator::new();
    let managed_git = Arc::new(ManagedGitScheduler::without_durable_state(work.clone()));
    vaults
        .reconcile_and_reconstruct(&registry, &committed, &work, &managed_git)
        .await;
    let executor = VaultWorkExecutor {
        vaults,
        registry,
        work,
        managed_git,
        commit_cooldown: Arc::new(crate::git::CommitCooldown::new()),
        cache: Arc::new(SqliteCache::in_memory(384).expect("open shared cache")),
        embedder: Arc::new(StubEmbedder::new(384)),
        runtime_config: RuntimeConfig::for_tests(),
        startup: StartupTracker::scanning(),
        model_setup_started: Arc::new(AtomicBool::new(true)),
        index_retries: IndexRetries::default(),
        index_slice: INDEX_TURN_SLICE,
    };
    (executor, worker, first, second, second_path)
}

/// The audit's first finding: one Vault's failed Index turn latched the
/// instance-wide tracker `Unavailable`, so `/ready` answered 503 although the
/// other Vault was indexed and serving (#326).
#[tokio::test]
async fn one_vaults_failed_index_turn_leaves_the_collection_ready() {
    let directory = tempdir().expect("temporary state directory");
    let (executor, mut worker, first, second, second_path) =
        two_vault_executor(directory.path()).await;
    // The second Vault's directory goes away after activation, so its scan
    // fails for real.
    std::fs::remove_dir_all(&second_path).expect("remove second Vault directory");

    for _ in 0..2 {
        let outcome = worker
            .run_next(|request| executor.run(request))
            .await
            .expect("reconstructed Index turn");
        executor.publish_outcome(&outcome);
        match outcome.request.vault_id() {
            vault_id if vault_id == first => outcome.result.expect("the healthy Vault indexes"),
            vault_id => {
                assert_eq!(vault_id, second);
                assert_eq!(
                    outcome.result.expect_err("the broken Vault fails").code(),
                    "vault_index_failed"
                );
            }
        }
    }

    assert!(
        executor.startup.collection_indexes_ready(),
        "the healthy Vault is serving, so the instance is ready"
    );
    assert_eq!(executor.startup.status().state, "ready");
    assert!(!executor.model_setup_started.load(Ordering::Acquire));
    let failed = executor
        .vaults
        .runtime(second)
        .expect("second Vault")
        .snapshot();
    assert_eq!(failed.search, VaultSearchStatus::Unavailable);
    assert_eq!(
        failed
            .search_error
            .expect("the failure is on the Vault")
            .code,
        "vault_index_failed",
        "the failure stays visible on the Vault that had it"
    );
    assert_eq!(
        executor
            .vaults
            .runtime(first)
            .expect("first Vault")
            .snapshot()
            .search,
        VaultSearchStatus::Ready
    );
}

/// A routine reindex after the collection settled is one Vault's upkeep and
/// reports on that Vault's own status; it must not move the instance tracker, which
/// `/ready` reads, back out of `Ready` (#326).
#[tokio::test]
async fn a_routine_reindex_does_not_leave_startup_readiness() {
    let directory = tempdir().expect("temporary state directory");
    let (executor, mut worker, first, _second, _) = two_vault_executor(directory.path()).await;
    for _ in 0..2 {
        let outcome = worker
            .run_next(|request| executor.run(request))
            .await
            .expect("reconstructed Index turn");
        executor.publish_outcome(&outcome);
        outcome.result.expect("Index turn succeeds");
    }
    assert!(executor.startup.collection_indexes_ready());

    std::fs::write(
        directory.path().join("first/Three.md"),
        "# Three\n\na change the watcher would report",
    )
    .expect("write a new note");
    assert_eq!(
        executor.work.request(first, VaultWorkKind::Index),
        ScheduleResult::Queued
    );
    let startup = executor.startup.clone();
    let observed_ready_throughout = Arc::new(AtomicBool::new(true));
    let observer = observed_ready_throughout.clone();
    let executor_ref = &executor;
    let outcome = worker
        .run_next(|request| async move {
            let executor = executor_ref;
            // Progress is reported from inside the turn; readiness must hold
            // at every point of it, not only once it has finished.
            let result = executor.run(request).await;
            if !startup.collection_indexes_ready() {
                observer.store(false, Ordering::Release);
            }
            result
        })
        .await
        .expect("routine Index turn");
    outcome.result.expect("routine reindex succeeds");
    assert!(observed_ready_throughout.load(Ordering::Acquire));
    assert_eq!(executor.startup.status().state, "ready");
}

/// A retryable Index failure asks for another turn after a backoff that
/// doubles, and stops after a bounded number of attempts. Before #326 a
/// failed turn was never retried on its own.
#[tokio::test(start_paused = true)]
async fn a_retryable_index_failure_is_retried_with_a_bounded_backoff() {
    let directory = tempdir().expect("temporary state directory");
    let (executor, mut worker, first, second, _) = two_vault_executor(directory.path()).await;
    // Clear the reconstructed turns so only a retry can queue work.
    for _ in 0..2 {
        worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .expect("reconstructed turn");
    }
    let failure = VaultWorkOutcome {
        request: VaultWorkRequest::for_tests(first, VaultWorkKind::Index),
        result: Err(VaultWorkError::new(
            "vault_index_failed",
            "scan failed",
            true,
        )),
    };

    let mut delay = INDEX_RETRY_BASE_DELAY;
    for attempt in 0..INDEX_RETRY_LIMIT {
        executor.publish_outcome(&failure);
        tokio::time::sleep(delay - Duration::from_secs(1)).await;
        assert!(
            !executor.work.has_work(first, VaultWorkKind::Index),
            "attempt {attempt} waits out its backoff"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(
            executor.work.has_work(first, VaultWorkKind::Index),
            "attempt {attempt} is requested once its backoff elapses"
        );
        worker
            .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
            .await
            .expect("retried turn");
        delay *= 2;
    }

    executor.publish_outcome(&failure);
    tokio::time::sleep(delay * 2).await;
    assert!(
        !executor.work.has_work(first, VaultWorkKind::Index),
        "retries stop once the limit is spent"
    );

    // A success resets the count, and a non-retryable failure asks for
    // nothing.
    executor.publish_outcome(&VaultWorkOutcome {
        request: failure.request,
        result: Ok(()),
    });
    executor.publish_outcome(&failure);
    tokio::time::sleep(INDEX_RETRY_BASE_DELAY + Duration::from_secs(1)).await;
    assert!(executor.work.has_work(first, VaultWorkKind::Index));
    worker
        .run_next(|_| async { Ok::<(), VaultWorkError>(()) })
        .await
        .expect("retried turn");
    executor.publish_outcome(&VaultWorkOutcome {
        request: VaultWorkRequest::for_tests(second, VaultWorkKind::Index),
        result: Err(VaultWorkError::new("vault_index_failed", "for good", false)),
    });
    tokio::time::sleep(INDEX_RETRY_BASE_DELAY * 4).await;
    assert!(!executor.work.has_work(second, VaultWorkKind::Index));
}

/// A panic in a turn's async shell is caught by the worker (#326). The turn
/// never reached its own failure publication, so the executor publishes it:
/// otherwise the Vault would read `Indexing` forever. And the Vault can still
/// be disabled afterwards without the request hanging.
#[tokio::test]
async fn a_panicking_index_turn_is_published_and_its_vault_can_still_be_disabled() {
    let directory = tempdir().expect("temporary state directory");
    let (executor, mut worker, _first, _second, _) = two_vault_executor(directory.path()).await;

    let outcome = worker
        .run_next(|request| {
            let vaults = executor.vaults.clone();
            async move {
                vaults
                    .runtime(request.vault_id())
                    .expect("active Vault")
                    .set_search_status(VaultSearchStatus::Indexing, None)
                    .expect("publish indexing");
                panic!("injected panic in the turn's async shell");
            }
        })
        .await
        .expect("the panicking turn still completes");
    executor.publish_outcome(&outcome);
    let panicked = outcome.request.vault_id();
    let snapshot = executor
        .vaults
        .runtime(panicked)
        .expect("panicked Vault")
        .snapshot();
    assert_eq!(snapshot.search, VaultSearchStatus::Unavailable);
    assert_eq!(
        snapshot
            .search_error
            .expect("the panic is on the Vault")
            .code,
        crate::vault_work::TURN_PANICKED
    );

    // The other Vault's turn still runs.
    let next = worker
        .run_next(|request| executor.run(request))
        .await
        .expect("the other Vault's turn");
    assert_ne!(next.request.vault_id(), panicked);
    next.result.expect("the other Vault indexes");

    let current = match executor.registry.load().expect("load registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let disabled = executor
        .registry
        .disable(current.revision(), panicked)
        .expect("disable the panicked Vault");
    tokio::time::timeout(
        Duration::from_secs(5),
        executor.vaults.reconcile_and_reconstruct(
            &executor.registry,
            &disabled,
            &executor.work,
            &executor.managed_git,
        ),
    )
    .await
    .expect("disabling the Vault whose turn panicked does not hang");
    assert!(executor.vaults.runtime(panicked).is_none());
}

#[tokio::test]
async fn a_turn_that_panics_holding_its_vaults_status_lock_does_not_stop_the_dispatch_loop() {
    let directory = tempdir().expect("temporary state directory");
    let (executor, mut worker, _first, _second, _) = two_vault_executor(directory.path()).await;

    let outcome = worker
        .run_next(|request| {
            let vaults = executor.vaults.clone();
            async move {
                vaults
                    .runtime(request.vault_id())
                    .expect("active Vault")
                    .while_holding_status_lock(|| {
                        panic!("injected panic while holding the Vault's status lock")
                    })
            }
        })
        .await
        .expect("the panicking turn still completes");
    // The dispatch loop calls this right after the turn; with the status lock
    // poisoned it used to panic out of the loop and end every Vault's work.
    executor.publish_outcome(&outcome);
    let panicked = outcome.request.vault_id();
    let snapshot = executor
        .vaults
        .runtime(panicked)
        .expect("panicked Vault")
        .snapshot();
    assert_eq!(snapshot.search, VaultSearchStatus::Unavailable);
    assert_eq!(
        snapshot
            .search_error
            .expect("the panic is on the Vault")
            .code,
        crate::vault_work::TURN_PANICKED
    );

    // The loop goes on: the other Vault indexes and the collection settles.
    let next = worker
        .run_next(|request| executor.run(request))
        .await
        .expect("the other Vault's turn");
    assert_ne!(next.request.vault_id(), panicked);
    next.result.as_ref().expect("the other Vault indexes");
    executor.publish_outcome(&next);
    assert!(executor.startup.collection_indexes_ready());
}

/// Drive one Git turn for `vault_id` through `dispatch_git_turn`.
async fn run_one_git_turn(
    collection: &VaultCollectionRuntime,
    registry: &VaultRegistryStore,
    coordinator: &VaultWorkCoordinator,
    managed_git: &ManagedGitScheduler,
    worker: &mut crate::vault_work::VaultWorkWorker,
    vault_id: VaultId,
) -> Result<(), VaultWorkError> {
    coordinator.request(vault_id, VaultWorkKind::Git);
    worker
        .run_next(|request| {
            dispatch_git_turn(
                collection,
                registry,
                coordinator,
                managed_git,
                "Hatchdoor",
                "hatchdoor@example.test",
                request,
            )
        })
        .await
        .expect("Git turn dequeued")
        .result
}

async fn run_one_recovery_turn(
    collection: &VaultCollectionRuntime,
    registry: &VaultRegistryStore,
    coordinator: &VaultWorkCoordinator,
    managed_git: &ManagedGitScheduler,
    worker: &mut crate::vault_work::VaultWorkWorker,
    vault_id: VaultId,
) -> Result<(), VaultWorkError> {
    coordinator.request(vault_id, VaultWorkKind::Recovery);
    worker
        .run_next(|request| {
            assert_eq!(request.kind(), VaultWorkKind::Recovery);
            dispatch_recovery_turn(
                collection,
                registry,
                managed_git,
                "Hatchdoor",
                "hatchdoor@example.test",
                request,
            )
        })
        .await
        .expect("recovery turn dequeued")
        .result
}

fn push_as_actor(remote_path: &Path, actor_path: &Path, path: &str, contents: &str) {
    let actor = git2::Repository::clone(remote_path.to_str().expect("remote path"), actor_path)
        .expect("actor checkout");
    commit_file(&actor, path, contents, "their change");
    actor
        .find_remote("origin")
        .expect("origin")
        .push(&["refs/heads/master:refs/heads/master"], None)
        .expect("actor push");
}

fn remote_ref(remote_path: &Path, reference: &str) -> Option<git2::Oid> {
    git2::Repository::open_bare(remote_path)
        .expect("remote")
        .refname_to_id(reference)
        .ok()
}

/// ADR-30 end to end through the executor: a Two-way Vault stops on a
/// conflict, a recovery turn publishes its side without touching the Vault's
/// Git status or the configured branch, and the sync that follows a
/// resolution on the Git host clears the report while the branch stays.
#[tokio::test]
async fn a_recovery_turn_publishes_the_vaults_side_and_the_next_sync_after_a_resolution_clears_it()
{
    let directory = tempdir().expect("temporary state directory");
    let (repository_path, remote_path) = existing_git_checkout_fixture(directory.path());
    let (collection, registry, control_block, vault_id) = existing_git_control_block(
        directory.path(),
        "Conflicted",
        repository_path.clone(),
        VaultGitMode::TwoWay,
    );
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());

    push_as_actor(
        &remote_path,
        &directory.path().join("actor"),
        "vault/Home.md",
        "theirs\n",
    );
    std::fs::write(repository_path.join("vault/Home.md"), "mine\n").expect("local edit");
    let conflict = run_one_git_turn(
        &collection,
        &registry,
        &coordinator,
        &managed_git,
        &mut worker,
        vault_id,
    )
    .await
    .expect_err("the sync conflicts");
    assert_eq!(conflict.code(), "managed_git_conflict");
    let conflicted = control_block.snapshot();
    assert!(conflicted.capabilities.publish_recovery);
    assert!(conflicted.recovery_branch.is_none());
    let remote_master = remote_ref(&remote_path, "refs/heads/master");

    run_one_recovery_turn(
        &collection,
        &registry,
        &coordinator,
        &managed_git,
        &mut worker,
        vault_id,
    )
    .await
    .expect("the recovery branch is published");

    let branch = format!("hatchdoor-recovery/master/{vault_id}");
    let local_head = git2::Repository::open(&repository_path)
        .expect("checkout")
        .refname_to_id("refs/heads/master")
        .expect("local head");
    let published = control_block.snapshot();
    let status = published.recovery_branch.expect("recovery status");
    assert_eq!(status.branch.as_deref(), Some(branch.as_str()));
    assert_eq!(status.published_commit, Some(local_head.to_string()));
    assert_eq!(
        status.conflicting_commit,
        remote_master.map(|oid| oid.to_string())
    );
    assert!(status.published_at.is_some());
    assert!(status.error.is_none());
    assert_eq!(
        published.git_error.map(|error| error.code),
        Some("managed_git_conflict".to_string()),
        "the conflict stays the Vault's Git failure until a sync resolves it"
    );
    assert_eq!(
        remote_ref(&remote_path, &format!("refs/heads/{branch}")),
        Some(local_head)
    );
    assert_eq!(
        remote_ref(&remote_path, "refs/heads/master"),
        remote_master,
        "the configured branch on the remote is untouched"
    );

    // Resolve on the Git host by merging the recovery branch into master.
    let resolver_path = directory.path().join("resolver");
    let resolver =
        git2::Repository::clone(remote_path.to_str().expect("remote path"), &resolver_path)
            .expect("resolver checkout");
    let ours = resolver.refname_to_id("refs/heads/master").expect("master");
    let theirs = resolver
        .refname_to_id(&format!("refs/remotes/origin/{branch}"))
        .expect("recovery branch");
    std::fs::write(resolver_path.join("vault/Home.md"), "resolved\n").expect("resolve");
    let mut index = resolver.index().expect("index");
    index.add_path(Path::new("vault/Home.md")).expect("stage");
    index.write().expect("write index");
    let tree = resolver
        .find_tree(index.write_tree().expect("tree"))
        .expect("tree");
    let signature = git2::Signature::now("Resolver", "r@example.test").expect("signature");
    resolver
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "Merge recovery branch",
            &tree,
            &[
                &resolver.find_commit(ours).expect("ours"),
                &resolver.find_commit(theirs).expect("theirs"),
            ],
        )
        .expect("merge commit");
    resolver
        .find_remote("origin")
        .expect("origin")
        .push(&["refs/heads/master:refs/heads/master"], None)
        .expect("push the resolution");

    run_one_git_turn(
        &collection,
        &registry,
        &coordinator,
        &managed_git,
        &mut worker,
        vault_id,
    )
    .await
    .expect("the sync resumes on its own");
    let resolved = control_block.snapshot();
    assert!(resolved.git_error.is_none());
    assert!(resolved.recovery_branch.is_none());
    assert!(!resolved.capabilities.publish_recovery);
    assert_eq!(
        std::fs::read_to_string(repository_path.join("vault/Home.md")).expect("note"),
        "resolved\n"
    );
    assert_eq!(
        remote_ref(&remote_path, &format!("refs/heads/{branch}")),
        Some(theirs),
        "Hatchdoor never deletes the recovery branch"
    );
}

/// A publish is a Git turn like any other: it waits for a foreground write
/// holding the Vault's mutation lock instead of pushing mid-write (ADR-18).
#[tokio::test]
async fn a_recovery_turn_waits_for_a_concurrent_foreground_mutation_to_release_the_lock() {
    let directory = tempdir().expect("temporary state directory");
    let (repository_path, remote_path) = existing_git_checkout_fixture(directory.path());
    let (collection, registry, control_block, vault_id) = existing_git_control_block(
        directory.path(),
        "Locked",
        repository_path.clone(),
        VaultGitMode::TwoWay,
    );
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
    push_as_actor(
        &remote_path,
        &directory.path().join("actor"),
        "vault/Home.md",
        "theirs\n",
    );
    std::fs::write(repository_path.join("vault/Home.md"), "mine\n").expect("local edit");
    run_one_git_turn(
        &collection,
        &registry,
        &coordinator,
        &managed_git,
        &mut worker,
        vault_id,
    )
    .await
    .expect_err("the sync conflicts");

    let mutation_guard = control_block
        .acquire_mutation()
        .await
        .expect("foreground mutation lock");
    coordinator.request(vault_id, VaultWorkKind::Recovery);
    let dispatch = worker.run_next(|request| {
        dispatch_recovery_turn(
            &collection,
            &registry,
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
        "the recovery turn must block on the foreground mutation lock"
    );
    assert!(
        remote_ref(
            &remote_path,
            &format!("refs/heads/hatchdoor-recovery/master/{vault_id}")
        )
        .is_none(),
        "nothing was pushed while the write held the lock"
    );
    drop(mutation_guard);
    tokio::time::timeout(std::time::Duration::from_secs(5), dispatch)
        .await
        .expect("the recovery turn proceeds once the lock is released")
        .expect("recovery turn dequeued")
        .result
        .expect("the recovery branch is published");
}

/// A request admitted during a conflict that a sync resolved before the
/// request's turn came up publishes nothing and says why on the status.
#[tokio::test]
async fn a_recovery_turn_for_a_vault_no_longer_in_conflict_publishes_nothing() {
    let directory = tempdir().expect("temporary state directory");
    let (repository_path, remote_path) = existing_git_checkout_fixture(directory.path());
    let (collection, registry, control_block, vault_id) = existing_git_control_block(
        directory.path(),
        "Healthy",
        repository_path,
        VaultGitMode::TwoWay,
    );
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());

    let error = run_one_recovery_turn(
        &collection,
        &registry,
        &coordinator,
        &managed_git,
        &mut worker,
        vault_id,
    )
    .await
    .expect_err("nothing to publish");

    assert_eq!(error.code(), "capability_unavailable");
    let status = control_block
        .snapshot()
        .recovery_branch
        .expect("the refusal is reported");
    assert_eq!(
        status.error.map(|error| error.code),
        Some("capability_unavailable".to_string())
    );
    assert!(status.published_commit.is_none());
    assert!(
        remote_ref(
            &remote_path,
            &format!("refs/heads/hatchdoor-recovery/master/{vault_id}")
        )
        .is_none()
    );
}

/// A refused publish reports on `recovery_branch` and nowhere else: the
/// earlier publication's fields stay, because that branch still stands, and
/// the conflict stays the Vault's Git failure (ADR-30).
#[tokio::test]
async fn a_refused_recovery_publish_keeps_the_earlier_publication_and_the_conflict() {
    let directory = tempdir().expect("temporary state directory");
    let (repository_path, remote_path) = existing_git_checkout_fixture(directory.path());
    let (collection, registry, control_block, vault_id) = existing_git_control_block(
        directory.path(),
        "Diverged",
        repository_path.clone(),
        VaultGitMode::TwoWay,
    );
    let (coordinator, mut worker) = VaultWorkCoordinator::new();
    let managed_git = ManagedGitScheduler::without_durable_state(coordinator.clone());
    push_as_actor(
        &remote_path,
        &directory.path().join("actor"),
        "vault/Home.md",
        "theirs\n",
    );
    std::fs::write(repository_path.join("vault/Home.md"), "mine\n").expect("local edit");
    run_one_git_turn(
        &collection,
        &registry,
        &coordinator,
        &managed_git,
        &mut worker,
        vault_id,
    )
    .await
    .expect_err("the sync conflicts");
    run_one_recovery_turn(
        &collection,
        &registry,
        &coordinator,
        &managed_git,
        &mut worker,
        vault_id,
    )
    .await
    .expect("first publish");
    let first = control_block.snapshot().recovery_branch.expect("published");

    // Someone starts resolving on the recovery branch itself.
    let branch = format!("hatchdoor-recovery/master/{vault_id}");
    let resolver_path = directory.path().join("resolver");
    let resolver =
        git2::Repository::clone(remote_path.to_str().expect("remote path"), &resolver_path)
            .expect("resolver checkout");
    let tip = resolver
        .refname_to_id(&format!("refs/remotes/origin/{branch}"))
        .expect("recovery branch");
    resolver
        .branch("work", &resolver.find_commit(tip).expect("tip"), false)
        .expect("work branch");
    resolver.set_head("refs/heads/work").expect("switch");
    resolver
        .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .expect("checkout");
    commit_file(&resolver, "vault/Home.md", "half resolved\n", "resolving");
    resolver
        .find_remote("origin")
        .expect("origin")
        .push(
            &[format!("refs/heads/work:refs/heads/{branch}").as_str()],
            None,
        )
        .expect("push to the recovery branch");
    std::fs::write(repository_path.join("vault/Later.md"), "later\n").expect("later save");

    let error = run_one_recovery_turn(
        &collection,
        &registry,
        &coordinator,
        &managed_git,
        &mut worker,
        vault_id,
    )
    .await
    .expect_err("diverged");

    assert_eq!(error.code(), "managed_git_recovery_diverged");
    let after = control_block.snapshot();
    let status = after.recovery_branch.expect("refusal reported");
    assert_eq!(
        status.error.map(|error| error.code),
        Some("managed_git_recovery_diverged".to_string())
    );
    assert_eq!(status.branch, first.branch);
    assert_eq!(status.published_commit, first.published_commit);
    assert_eq!(status.published_at, first.published_at);
    assert_eq!(
        after.git_error.map(|error| error.code),
        Some("managed_git_conflict".to_string())
    );
    assert!(after.capabilities.publish_recovery);
}

/// Two Vaults through the real executor: once the first Vault's turn has
/// finished, the startup reading covers the second one still queued instead
/// of claiming 100% for the first alone (#373).
#[tokio::test]
async fn first_run_progress_through_the_executor_covers_the_queued_vault() {
    let directory = tempdir().expect("temporary state directory");
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    for (path, prefix) in [(&first_path, "First"), (&second_path, "Second")] {
        std::fs::create_dir_all(path).expect("Vault directory");
        for index in 0..3 {
            std::fs::write(
                path.join(format!("{prefix} {index}.md")),
                format!("# {prefix} {index}\n\nA note about sleep and circadian rhythm."),
            )
            .expect("write note");
        }
    }
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let with_first = add_local_vault(&registry, &empty, "First", first_path);
    let committed = add_local_vault(&registry, &with_first, "Second", second_path);

    let vaults = VaultCollectionRuntime::new();
    let (work, mut worker) = VaultWorkCoordinator::new();
    let managed_git = Arc::new(ManagedGitScheduler::without_durable_state(work.clone()));
    vaults
        .reconcile_and_reconstruct(&registry, &committed, &work, &managed_git)
        .await;
    let executor = VaultWorkExecutor {
        vaults: vaults.clone(),
        registry: registry.clone(),
        work: work.clone(),
        managed_git: managed_git.clone(),
        commit_cooldown: Arc::new(crate::git::CommitCooldown::new()),
        cache: Arc::new(SqliteCache::in_memory(384).expect("open shared cache")),
        embedder: Arc::new(StubEmbedder::new(384)),
        runtime_config: RuntimeConfig::for_tests(),
        startup: StartupTracker::scanning(),
        model_setup_started: Arc::new(AtomicBool::new(true)),
        index_retries: IndexRetries::default(),
        index_slice: INDEX_TURN_SLICE,
    };

    let outcome = worker
        .run_next(|request| executor.run(request))
        .await
        .expect("first Index turn");
    outcome.result.as_ref().expect("first Index turn succeeds");
    executor.publish_outcome(&outcome);

    let status = executor.startup.status();
    assert_eq!(status.state, "indexing");
    let percent = status.percent.expect("percent");
    assert!(
        (1..100).contains(&percent),
        "one of two equal Vaults done must read part-way, not {percent}%"
    );
    assert!(
        status.eta_seconds.is_some(),
        "time left still covers the queued Vault after the first one finished"
    );

    let outcome = worker
        .run_next(|request| executor.run(request))
        .await
        .expect("second Index turn");
    outcome.result.as_ref().expect("second Index turn succeeds");
    executor.publish_outcome(&outcome);
    assert_eq!(executor.startup.status().state, "ready");
}

/// Counting a queued Vault's notes reads directory entries only. It finishes
/// while that Vault's foreground mutation guard is held, so it cannot wait on
/// a write in progress or hold one up.
#[tokio::test]
async fn counting_a_queued_vaults_notes_does_not_take_its_mutation_guard() {
    let directory = tempdir().expect("temporary state directory");
    let first_path = directory.path().join("first");
    let queued_path = directory.path().join("queued");
    std::fs::create_dir_all(&first_path).expect("first Vault directory");
    std::fs::create_dir_all(queued_path.join("sub")).expect("queued Vault directory");
    std::fs::write(first_path.join("One.md"), "# One").expect("write note");
    std::fs::write(queued_path.join("A.md"), "# A").expect("write note");
    std::fs::write(queued_path.join("sub/B.md"), "# B").expect("write note");
    let registry = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let with_first = add_local_vault(&registry, &empty, "First", first_path);
    let committed = add_local_vault(&registry, &with_first, "Queued", queued_path);
    let first = vault_id_named(&committed, "First");
    let queued = vault_id_named(&committed, "Queued");
    let vaults = VaultCollectionRuntime::new();
    vaults.reconcile(&registry, &committed);

    let _held = vaults
        .runtime(queued)
        .expect("queued Vault")
        .acquire_mutation()
        .await
        .expect("hold the queued Vault's mutation guard");
    let startup = StartupTracker::scanning();
    report_first_run_progress(
        &startup,
        &vaults,
        first,
        IndexingProgressSnapshot {
            notes_total: 1,
            tokens_total: 10,
            ..IndexingProgressSnapshot::default()
        },
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while startup.recorded_note_count(queued).is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the note count never landed while the guard was held"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(startup.recorded_note_count(queued), Some(Some(2)));
    assert_eq!(
        startup.recorded_note_count(first),
        None,
        "the reporting Vault brings its own count"
    );
}

#[test]
fn note_count_honours_exclusions_and_reports_an_unreadable_vault() {
    let directory = tempdir().expect("temporary Vault directory");
    let root = directory.path();
    std::fs::create_dir_all(root.join("drafts")).expect("drafts directory");
    std::fs::write(root.join("Kept.md"), "").expect("write note");
    std::fs::write(root.join("drafts/Skipped.md"), "").expect("write note");
    std::fs::write(root.join("image.png"), "").expect("write asset");

    assert_eq!(count_markdown_notes(root, &[]), Some(2));
    assert_eq!(
        count_markdown_notes(root, &["drafts/".to_string()]),
        Some(1)
    );
    assert_eq!(count_markdown_notes(&root.join("missing"), &[]), None);
}

/// Counts every input it embeds, and sleeps on any input containing
/// [`SLOW_MARKER`], so a test can make one Vault's turn take wall time
/// without making the other's.
struct CountingEmbedder {
    inner: StubEmbedder,
    embedded: std::sync::atomic::AtomicUsize,
}

const SLOW_MARKER: &str = "SLOWNOTE";

impl Embedder for CountingEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if texts.iter().any(|text| text.contains(SLOW_MARKER)) {
            std::thread::sleep(Duration::from_millis(1_200));
        }
        self.embedded.fetch_add(texts.len(), Ordering::SeqCst);
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

const LARGE_NOTES: usize = 4;

/// A large Vault of [`LARGE_NOTES`] one-chunk notes and a small one-note
/// Vault, with an executor whose Index turns take turns after `slice`. The
/// coordinator starts empty, so each test queues the Vaults in the order it
/// needs, and each Vault's status follows the indexing lane as it does in
/// production.
async fn large_and_small_executor(
    directory: &Path,
    slice: Duration,
    small_note: &str,
) -> (
    VaultWorkExecutor,
    crate::vault_work::VaultWorkWorker,
    Arc<CountingEmbedder>,
    VaultId,
    VaultId,
) {
    let large_path = directory.join("large");
    let small_path = directory.join("small");
    std::fs::create_dir_all(&large_path).expect("large Vault directory");
    std::fs::create_dir_all(&small_path).expect("small Vault directory");
    for index in 0..LARGE_NOTES {
        std::fs::write(
            large_path.join(format!("Large {index}.md")),
            format!("# Large {index}\n\nA note about sleep, number {index}."),
        )
        .expect("write large note");
    }
    std::fs::write(small_path.join("Small.md"), small_note).expect("write small note");
    let registry = VaultRegistryStore::new(directory.join("state/vaults.json"));
    let empty = match registry.load().expect("load empty registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let with_large = add_local_vault(&registry, &empty, "Large", large_path);
    let committed = add_local_vault(&registry, &with_large, "Small", small_path);
    let large = vault_id_named(&committed, "Large");
    let small = vault_id_named(&committed, "Small");
    let vaults = VaultCollectionRuntime::new();
    // Reconstruction queues both Vaults in Vault ID order, which is random.
    // It queues them on a coordinator this test then throws away.
    let (reconstruction, _) = VaultWorkCoordinator::new();
    let (work, worker) = VaultWorkCoordinator::new();
    let managed_git = Arc::new(ManagedGitScheduler::without_durable_state(work.clone()));
    vaults
        .reconcile_and_reconstruct(&registry, &committed, &reconstruction, &managed_git)
        .await;
    report_index_lane_on_vault_status(&vaults, &work);
    let embedder = Arc::new(CountingEmbedder {
        inner: StubEmbedder::new(384),
        embedded: std::sync::atomic::AtomicUsize::new(0),
    });
    let executor = VaultWorkExecutor {
        vaults,
        registry,
        work,
        managed_git,
        commit_cooldown: Arc::new(crate::git::CommitCooldown::new()),
        cache: Arc::new(SqliteCache::in_memory(384).expect("open shared cache")),
        embedder: embedder.clone(),
        runtime_config: RuntimeConfig::for_tests(),
        startup: StartupTracker::scanning(),
        model_setup_started: Arc::new(AtomicBool::new(true)),
        index_retries: IndexRetries::default(),
        index_slice: slice,
    };
    (executor, worker, embedder, large, small)
}

/// Take the next turn, check which Vault it is for, and run it through the
/// executor the way the dispatch loop does.
async fn run_index_turn(
    executor: &VaultWorkExecutor,
    worker: &mut crate::vault_work::VaultWorkWorker,
    expected: VaultId,
) {
    let turn = worker.next_turn().await.expect("a queued Index turn");
    assert_eq!(turn.request().vault_id(), expected);
    assert_eq!(
        vault_status(executor, expected).index_turn,
        Some(VaultIndexTurn::Running),
        "a Vault whose turn holds the indexing slot reports it running"
    );
    let outcome = turn.run(|request| executor.run(request)).await;
    executor.publish_outcome(&outcome);
    outcome.result.expect("the Index turn does not fail");
}

fn vault_status(executor: &VaultWorkExecutor, vault_id: VaultId) -> CollectionVaultSnapshot {
    executor
        .vaults
        .runtime(vault_id)
        .expect("active Vault")
        .snapshot()
}

fn saved_vectors(cache: &SqliteCache, vault_id: VaultId) -> usize {
    let conn = cache.read().expect("read connection");
    conn.query_row(
        "SELECT COUNT(*) FROM vault_embedding_progress WHERE vault_id = ?1",
        [vault_id.to_string()],
        |row| row.get::<_, i64>(0),
    )
    .expect("count saved vectors") as usize
}

/// ADR-35 decisions 3 and 5: the large Vault pauses after its slice for the
/// small one queued behind it, says it is waiting, and resumes once the small
/// one has finished, embedding only what it had not saved.
#[tokio::test]
async fn a_long_index_turn_pauses_for_a_waiting_vault_and_resumes_without_reembedding() {
    let directory = tempdir().expect("temporary state directory");
    let (executor, mut worker, embedder, large, small) =
        large_and_small_executor(directory.path(), Duration::ZERO, "# Small\n\nsmall note").await;
    executor.work.request(large, VaultWorkKind::Index);
    executor.work.request(small, VaultWorkKind::Index);
    assert_eq!(
        vault_status(&executor, small).index_turn,
        Some(VaultIndexTurn::Waiting),
        "a queued Vault reports it is waiting for its turn"
    );

    run_index_turn(&executor, &mut worker, large).await;
    assert_eq!(
        embedder.embedded.load(Ordering::SeqCst),
        1,
        "with a zero slice the large Vault stops at its first chunk boundary"
    );
    let paused = vault_status(&executor, large);
    assert_eq!(paused.index_turn, Some(VaultIndexTurn::Waiting));
    assert_eq!(
        paused.search,
        VaultSearchStatus::Browsable,
        "a paused first build keeps its published notes browsable"
    );
    assert_eq!(paused.search_error, None, "a pause is not a failure");
    assert_eq!(saved_vectors(&executor.cache, large), 1);

    run_index_turn(&executor, &mut worker, small).await;
    let small_done = vault_status(&executor, small);
    assert_eq!(small_done.search, VaultSearchStatus::Ready);
    assert_eq!(small_done.index_turn, None);
    assert_eq!(
        vault_status(&executor, large).index_turn,
        Some(VaultIndexTurn::Waiting)
    );

    run_index_turn(&executor, &mut worker, large).await;
    let large_done = vault_status(&executor, large);
    assert_eq!(large_done.search, VaultSearchStatus::Ready);
    assert_eq!(large_done.index_turn, None);
    assert_eq!(
        embedder.embedded.load(Ordering::SeqCst),
        LARGE_NOTES + 1,
        "every chunk was embedded exactly once across both of the large Vault's turns"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(25), worker.next_turn())
            .await
            .is_err(),
        "nothing is left queued"
    );
}

/// ADR-35 decision 3: with nothing waiting, a turn carries on past every
/// slice, so a single-Vault instance never pays for taking turns.
#[tokio::test]
async fn an_index_turn_with_nothing_waiting_never_pauses() {
    let directory = tempdir().expect("temporary state directory");
    let (executor, mut worker, embedder, large, _small) =
        large_and_small_executor(directory.path(), Duration::ZERO, "# Small\n\nsmall note").await;
    executor.work.request(large, VaultWorkKind::Index);

    run_index_turn(&executor, &mut worker, large).await;

    assert_eq!(embedder.embedded.load(Ordering::SeqCst), LARGE_NOTES);
    let status = vault_status(&executor, large);
    assert_eq!(status.search, VaultSearchStatus::Ready);
    assert_eq!(status.index_turn, None);
    assert_eq!(executor.work.index_lane_state(large), None, "not requeued");
}

/// Disabling a paused Vault discards its place in the queue like any other
/// queued work, and keeps the progress it saved for when it is re-enabled.
#[tokio::test]
async fn disabling_a_paused_vault_discards_its_queued_turn_and_keeps_its_progress() {
    let directory = tempdir().expect("temporary state directory");
    let (executor, mut worker, _embedder, large, small) =
        large_and_small_executor(directory.path(), Duration::ZERO, "# Small\n\nsmall note").await;
    executor.work.request(large, VaultWorkKind::Index);
    executor.work.request(small, VaultWorkKind::Index);
    run_index_turn(&executor, &mut worker, large).await;
    assert_eq!(
        executor.work.index_lane_state(large),
        Some(crate::vault_work::IndexLaneState::Waiting)
    );

    let current = match executor.registry.load().expect("load registry") {
        crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
        crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("registry recovery"),
    };
    let disabled = executor
        .registry
        .disable(current.revision(), large)
        .expect("disable the large Vault");
    executor
        .vaults
        .reconcile_and_reconstruct(
            &executor.registry,
            &disabled,
            &executor.work,
            &executor.managed_git,
        )
        .await;

    assert_eq!(executor.work.index_lane_state(large), None);
    assert_eq!(saved_vectors(&executor.cache, large), 1);
    run_index_turn(&executor, &mut worker, small).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(25), worker.next_turn())
            .await
            .is_err(),
        "the disabled Vault's paused turn is gone"
    );
}

/// ADR-35 decision 5: the time a paused Vault spends waiting is not counted
/// as embedding time, so its estimate does not run while it waits, and
/// resuming starts from the work it saved rather than below it. The
/// first-run reading across both Vaults never moves backwards meanwhile.
#[tokio::test]
async fn a_paused_vaults_progress_holds_while_it_waits_and_resumes_where_it_stopped() {
    let directory = tempdir().expect("temporary state directory");
    let (executor, mut worker, _embedder, large, small) = large_and_small_executor(
        directory.path(),
        Duration::ZERO,
        &format!("# Small\n\n{SLOW_MARKER} takes a while to embed"),
    )
    .await;
    // Each report is tagged with the turn it came from, and the first-run
    // reading is sampled on every report, not only between turns.
    let reports: Arc<Mutex<Vec<(usize, VaultId, IndexingProgressSnapshot)>>> = Arc::default();
    let readings: Arc<Mutex<Vec<u8>>> = Arc::default();
    let report =
        |turn: usize, vault_id: VaultId| -> Arc<dyn Fn(IndexingProgressSnapshot) + Send + Sync> {
            let reports = reports.clone();
            let readings = readings.clone();
            let startup = executor.startup.clone();
            let vaults = executor.vaults.clone();
            Arc::new(move |progress| {
                reports
                    .lock()
                    .expect("reports")
                    .push((turn, vault_id, progress));
                report_first_run_progress(&startup, &vaults, vault_id, progress);
                if let Some(percent) = startup.status().percent {
                    readings.lock().expect("readings").push(percent);
                }
            })
        };
    let slicing = || {
        Some(IndexTurnSlicing {
            work: executor.work.clone(),
            slice: Duration::ZERO,
        })
    };
    executor.work.request(large, VaultWorkKind::Index);
    executor.work.request(small, VaultWorkKind::Index);
    for (turn, expected) in [large, small, large].into_iter().enumerate() {
        let queued = worker.next_turn().await.expect("queued Index turn");
        assert_eq!(queued.request().vault_id(), expected);
        let outcome = queued
            .run(|request| {
                dispatch_vault_index_turn_with_progress(
                    &executor.vaults,
                    executor.cache.clone(),
                    executor.embedder.clone(),
                    true,
                    Some(report(turn, request.vault_id())),
                    slicing(),
                    request,
                )
            })
            .await;
        outcome.result.as_ref().expect("Index turn");
        executor.publish_outcome(&outcome);
        if let Some(percent) = executor.startup.status().percent {
            readings.lock().expect("readings").push(percent);
        }
    }

    let reports = reports.lock().expect("reports");
    let before_pause = reports
        .iter()
        .rev()
        .find(|(turn, _, _)| *turn == 0)
        .map(|(_, _, progress)| *progress)
        .expect("the large Vault's last report before it paused");
    let on_resume = reports
        .iter()
        .find(|(turn, _, _)| *turn == 2)
        .map(|(_, _, progress)| *progress)
        .expect("the large Vault's first report after it resumed");
    assert_eq!(
        before_pause.chunks_completed, 1,
        "it paused after one chunk"
    );
    assert!(
        on_resume.tokens_completed >= before_pause.tokens_completed
            && on_resume.tokens_total == before_pause.tokens_total,
        "resuming starts from the saved work: {before_pause:?} then {on_resume:?}"
    );
    assert!(
        on_resume.elapsed_seconds <= before_pause.elapsed_seconds,
        "the 1.2s the large Vault waited for the small one is not embedding time: \
         {before_pause:?} then {on_resume:?}"
    );
    let readings = readings.lock().expect("readings");
    assert!(
        readings.windows(2).all(|pair| pair[0] <= pair[1]),
        "the first-run reading never moves backwards: {readings:?}"
    );
    assert_eq!(executor.startup.status().state, "ready");
}

/// Startup order: reconstruction queues every Vault's first Index turn
/// before the executor, and with it the status observer, exists. The
/// Vaults already queued must still say they are waiting.
#[tokio::test]
async fn vaults_queued_before_the_executor_starts_report_waiting() {
    let directory = tempdir().expect("temporary state directory");
    let (executor, _worker, _embedder, large, small) =
        large_and_small_executor(directory.path(), Duration::ZERO, "# Small\n\nsmall note").await;
    let (work, _worker) = VaultWorkCoordinator::new();
    work.request(large, VaultWorkKind::Index);
    work.request(small, VaultWorkKind::Index);
    assert_eq!(vault_status(&executor, small).index_turn, None);

    report_index_lane_on_vault_status(&executor.vaults, &work);

    for vault_id in [large, small] {
        assert_eq!(
            vault_status(&executor, vault_id).index_turn,
            Some(VaultIndexTurn::Waiting)
        );
    }
}
