//! Vault-qualified search over the published disposable cache snapshots.

use schemars::JsonSchema;
use std::cmp::Ordering;
use std::collections::BTreeMap;

#[cfg(test)]
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::cache::{
    SqliteCache,
    vault_snapshots::{NoteBodies, VaultSnapshotRead},
};
use crate::embed::Embedder;
use crate::vault::{NoteMetadata, NoteSummary};
use crate::vault_read::{
    BrowseSurface, VaultParticipant, VaultParticipantState, VaultReadError, VaultReadProjection,
    VaultScope, selected_vaults,
};
use crate::vault_registry::VaultId;
use crate::vault_runtime::VaultCollectionRuntime;

use super::{LayerSelection, OutboundLink, SearchMode, tag_prefix_query};

/// First KNN candidate window per requested result. The per-note cap is
/// applied after ranking, so a small cap must not shrink the first window to
/// the point where repeated chunks from one note crowd out eligible notes.
const INITIAL_DIVERSITY_OVERFETCH: usize = 4;

/// Search does not make unbounded KNN requests while backfilling after the
/// per-note cap. If this candidate ceiling is all from capped notes, callers
/// receive the best available cap-compliant partial result set.
const MAX_SEMANTIC_CANDIDATES: usize = 200;

#[cfg(test)]
type SnapshotReadHook = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug, Clone)]
pub struct VaultSearchRequest {
    pub scope: VaultScope,
    pub query: String,
    pub mode: SearchMode,
    pub limit: usize,
    pub per_note_cap: usize,
    pub layers: LayerSelection,
}

#[derive(Debug, Clone, Serialize, JsonSchema, Deserialize)]
pub struct VaultSearchResult {
    pub vault_id: VaultId,
    pub chunk_id: i64,
    pub note_slug: String,
    pub note_title: String,
    pub note_path: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub score: f32,
    pub layer: Option<String>,
    pub outbound_links: Vec<OutboundLink>,
    pub metadata: crate::vault::NoteMetadata,
}

#[derive(Debug, Clone, Serialize, JsonSchema, Deserialize)]
pub struct VaultSearchResponse {
    pub mode: SearchMode,
    pub results: Vec<VaultSearchResult>,
}

/// Search shared-core over published #91 snapshots. It deliberately contains
/// no handler, protocol, selected-Vault, or Markdown exact-read behavior.
pub struct VaultSearchCore<'a> {
    cache: &'a SqliteCache,
    vaults: &'a VaultCollectionRuntime,
    embedder: &'a dyn Embedder,
    surface: BrowseSurface,
    #[cfg(test)]
    after_snapshot_read_hook: Option<SnapshotReadHook>,
}

impl<'a> VaultSearchCore<'a> {
    pub fn new(
        cache: &'a SqliteCache,
        vaults: &'a VaultCollectionRuntime,
        embedder: &'a dyn Embedder,
    ) -> Self {
        Self {
            cache,
            vaults,
            embedder,
            surface: BrowseSurface::Everything,
            #[cfg(test)]
            after_snapshot_read_hook: None,
        }
    }

    /// Restrict this search to the default surface. The caller also clamps the
    /// request's own [`LayerSelection`], which keeps demoted chunks out of the
    /// hits; this additionally keeps them out of a surviving hit's outbound
    /// links, where a demoted Note would otherwise still be named (#109).
    pub fn on_surface(mut self, surface: BrowseSurface) -> Self {
        self.surface = surface;
        self
    }

    #[cfg(test)]
    fn with_after_snapshot_read_hook(mut self, hook: SnapshotReadHook) -> Self {
        self.after_snapshot_read_hook = Some(hook);
        self
    }

    pub fn search(
        &self,
        mut request: VaultSearchRequest,
    ) -> Result<VaultReadProjection<VaultSearchResponse>, VaultReadError> {
        request.query = request.query.trim().to_string();
        if request.query.is_empty() {
            return Err(error(
                None,
                "invalid_search_query",
                "query cannot be empty",
                false,
            ));
        }
        let collection = self.vaults.snapshot();
        let selected = selected_vaults(&collection, request.scope)?;
        // Resolved before the read lease because it decides whether this query
        // needs vectors at all: a tag query is answered from structural rows,
        // so a Vault without vectors is a full participant in it.
        let tag_query = tag_prefix_query(&request.query);
        let needs_vectors = tag_query.is_none() && request.mode == SearchMode::Semantic;
        // Embed before checking out a read lease, never while holding one.
        // Inference runs behind the embedder's own Mutex, so concurrent
        // semantic searches queue on it one at a time. A lease held across
        // that wait pins one of the cache's few reader slots while doing no
        // database work at all: four typed-ahead searches were enough to hold
        // every slot and answer `/tree` and `/recent` with an instant 503.
        //
        // Two things move ahead of the snapshot read as a result, both of
        // them visible only when the embedder itself is unhealthy. A query
        // whose Vaults turn out to have no participating snapshot is embedded
        // anyway, and an unhealthy embedder now reports `search_unavailable`
        // where a bad layer name or an unavailable single Vault would have
        // been named first. Both need the snapshot to detect, so keeping that
        // order would mean holding the lease across inference again.
        let query_vector = if needs_vectors && request.limit > 0 && request.per_note_cap > 0 {
            Some(embed_query(self.embedder, &request.query)?)
        } else {
            None
        };
        let mut cache_snapshot = self
            .cache
            .read_snapshot()
            .map_err(|message| error(None, "search_unavailable", &message, true))?;
        let mut snapshots = BTreeMap::new();
        let mut participants = Vec::with_capacity(selected.len());
        for vault in selected {
            match SqliteCache::read_vault_snapshot_on(
                &cache_snapshot,
                vault.vault_id,
                NoteBodies::Omit,
            ) {
                Ok(Some(snapshot)) => {
                    let state = match snapshot.status.freshness {
                        crate::cache::vault_snapshots::VaultSnapshotFreshness::Fresh => {
                            VaultParticipantState::Fresh
                        }
                        crate::cache::vault_snapshots::VaultSnapshotFreshness::Stale => {
                            VaultParticipantState::Stale
                        }
                    };
                    // A structure-only generation has chunk text but no
                    // vectors, so it answers keyword and tag queries in full
                    // and contributes nothing semantic. Say so rather than
                    // letting the per-chunk vector check drop it silently,
                    // which reads to a caller as "no matches here".
                    let state = if needs_vectors
                        && !snapshot.status.searchable
                        && state == VaultParticipantState::Fresh
                    {
                        VaultParticipantState::NotSearchable
                    } else {
                        state
                    };
                    participants.push(VaultParticipant {
                        vault_id: vault.vault_id,
                        vault_name: vault.vault_name,
                        state,
                        error: None,
                    });
                    snapshots.insert(vault.vault_id, self.surface.restrict(snapshot.read));
                }
                Ok(None) => self.push_unavailable(
                    request.scope,
                    &mut participants,
                    vault.vault_id,
                    vault.vault_name,
                    "Vault has no participating searchable snapshot",
                )?,
                Err(message) => self.push_unavailable(
                    request.scope,
                    &mut participants,
                    vault.vault_id,
                    vault.vault_name,
                    &message,
                )?,
            }
        }

        #[cfg(test)]
        if let Some(hook) = &self.after_snapshot_read_hook {
            hook();
        }

        validate_named_layers(&request.layers, snapshots.values())?;
        let results = if let Some(tag) = tag_query {
            tag_results(&request, &snapshots, &tag)
        } else {
            match request.mode {
                SearchMode::Semantic => match &query_vector {
                    Some(query_vector) => semantic_results(
                        &request,
                        self.cache,
                        &cache_snapshot,
                        query_vector,
                        &snapshots,
                    )?,
                    // `limit` or `per_note_cap` of zero asks for no results.
                    None => Vec::new(),
                },
                SearchMode::Keyword => {
                    let raw = keyword_hits(&request, self.cache, &cache_snapshot, &snapshots)?;
                    apply_per_note_cap(raw, request.per_note_cap, request.limit)
                }
            }
        };
        let response = VaultReadProjection {
            scope: request.scope,
            collection_revision: collection.collection_revision,
            partial: participants
                .iter()
                .any(|participant| participant.state != VaultParticipantState::Fresh),
            participants,
            data: VaultSearchResponse {
                mode: request.mode,
                results,
            },
        };
        cache_snapshot
            .commit()
            .map_err(|message| error(None, "search_unavailable", &message, true))?;
        Ok(response)
    }

    fn push_unavailable(
        &self,
        scope: VaultScope,
        participants: &mut Vec<VaultParticipant>,
        vault_id: VaultId,
        vault_name: String,
        message: &str,
    ) -> Result<(), VaultReadError> {
        let issue = error(Some(vault_id), "vault_unavailable", message, true);
        if matches!(scope, VaultScope::One(_)) {
            return Err(issue);
        }
        participants.push(VaultParticipant {
            vault_id,
            vault_name,
            state: VaultParticipantState::Unavailable,
            error: Some(issue),
        });
        Ok(())
    }
}

/// Embeds one search query. Callers run this before checking out a cache read
/// lease, never while holding one.
fn embed_query(embedder: &dyn Embedder, query: &str) -> Result<Vec<f32>, VaultReadError> {
    embedder
        .embed(&[format!("{}{}", embedder.query_prefix(), query)])
        .map_err(|message| error(None, "search_unavailable", &message, true))?
        .into_iter()
        .next()
        .ok_or_else(|| {
            error(
                None,
                "search_unavailable",
                "embedder returned no vectors",
                true,
            )
        })
}

fn semantic_results(
    request: &VaultSearchRequest,
    cache: &SqliteCache,
    conn: &rusqlite::Connection,
    query_vector: &[f32],
    snapshots: &BTreeMap<VaultId, VaultSnapshotRead>,
) -> Result<Vec<VaultSearchResult>, VaultReadError> {
    // `limit` and `per_note_cap` are the caller's to check: it gates the
    // embedding on them, so a zero never reaches here.
    if snapshots.is_empty() {
        return Ok(Vec::new());
    }

    progressively_cap_semantic_results(request.limit, request.per_note_cap, |raw_k| {
        semantic_hits_with_vector(request, cache, conn, snapshots, query_vector, raw_k)
    })
}

fn semantic_hits_with_vector(
    request: &VaultSearchRequest,
    cache: &SqliteCache,
    conn: &rusqlite::Connection,
    snapshots: &BTreeMap<VaultId, VaultSnapshotRead>,
    query_vector: &[f32],
    raw_k: usize,
) -> Result<Vec<VaultSearchResult>, VaultReadError> {
    if raw_k == 0 {
        return Ok(Vec::new());
    }
    let ids = snapshots.keys().copied().collect::<Vec<_>>();
    let hits = cache
        .vault_semantic_search_layered_with_vector(conn, &ids, query_vector, raw_k, &request.layers)
        .map_err(|message| error(None, "search_unavailable", &message, true))?;
    Ok(hits
        .into_iter()
        .filter_map(|hit| {
            let snapshot = snapshots.get(&hit.vault_id)?;
            let note = note_for(snapshot, &hit.note_slug)?;
            Some(result_for(
                hit.vault_id,
                snapshot,
                note,
                ResultDetails {
                    chunk_id: hit.chunk_id,
                    heading_path: hit.heading_path,
                    content: hit.content,
                    score: (1.0 - hit.distance).clamp(0.0, 1.0),
                },
            ))
        })
        .collect())
}

fn progressively_cap_semantic_results(
    limit: usize,
    per_note_cap: usize,
    mut fetch: impl FnMut(usize) -> Result<Vec<VaultSearchResult>, VaultReadError>,
) -> Result<Vec<VaultSearchResult>, VaultReadError> {
    let mut raw_k = limit
        .saturating_mul(per_note_cap.max(INITIAL_DIVERSITY_OVERFETCH))
        .min(MAX_SEMANTIC_CANDIDATES);
    loop {
        let raw = fetch(raw_k)?;
        let raw_len = raw.len();
        let results = apply_per_note_cap(raw, per_note_cap, limit);
        if results.len() == limit || raw_len < raw_k || raw_k == MAX_SEMANTIC_CANDIDATES {
            return Ok(results);
        }
        raw_k = raw_k.saturating_mul(2).min(MAX_SEMANTIC_CANDIDATES);
    }
}

fn keyword_hits(
    request: &VaultSearchRequest,
    cache: &SqliteCache,
    conn: &rusqlite::Connection,
    snapshots: &BTreeMap<VaultId, VaultSnapshotRead>,
) -> Result<Vec<VaultSearchResult>, VaultReadError> {
    let ids = snapshots.keys().copied().collect::<Vec<_>>();
    let raw = cache
        .vault_fts_search_chunks(conn, &ids, &request.query, &request.layers)
        .map_err(|message| error(None, "search_unavailable", &message, true))?;
    let max = raw
        .iter()
        .map(|hit| hit.bm25.abs())
        .fold(f32::NEG_INFINITY, f32::max);
    let mut hits = Vec::new();
    for hit in raw {
        let Some(snapshot) = snapshots.get(&hit.vault_id) else {
            continue;
        };
        let Some(note) = note_for(snapshot, &hit.note_slug) else {
            continue;
        };
        let score = if max <= f32::EPSILON {
            1.0
        } else {
            (hit.bm25.abs() / max).clamp(0.0, 1.0)
        };
        hits.push(result_for(
            hit.vault_id,
            snapshot,
            note,
            ResultDetails {
                chunk_id: hit.chunk_id,
                heading_path: hit.heading_path,
                content: hit.content,
                score,
            },
        ));
    }
    hits.sort_by(rank_order);
    Ok(hits)
}

fn tag_results(
    request: &VaultSearchRequest,
    snapshots: &BTreeMap<VaultId, VaultSnapshotRead>,
    tag: &str,
) -> Vec<VaultSearchResult> {
    let mut results = Vec::new();
    for (vault_id, snapshot) in snapshots {
        for note in &snapshot.notes {
            let summary = note_summary(note);
            if !layer_matches(&request.layers, note.layer.as_deref()) {
                continue;
            }
            if !note.metadata.tags.iter().any(|candidate| {
                candidate == tag
                    || candidate
                        .strip_prefix(tag)
                        .is_some_and(|tail| tail.starts_with('/'))
            }) {
                continue;
            }
            results.push(result_for(
                *vault_id,
                snapshot,
                summary,
                ResultDetails {
                    chunk_id: 0,
                    heading_path: None,
                    content: format!("Matched tag: #{tag}"),
                    score: 1.0,
                },
            ));
        }
    }
    results.sort_by(|left, right| {
        left.note_path
            .cmp(&right.note_path)
            .then_with(|| left.vault_id.cmp(&right.vault_id))
            .then_with(|| left.note_slug.cmp(&right.note_slug))
    });
    results.truncate(request.limit);
    results
}

fn validate_named_layers<'a>(
    selection: &LayerSelection,
    snapshots: impl Iterator<Item = &'a VaultSnapshotRead>,
) -> Result<(), VaultReadError> {
    let names = selection.named_layers();
    if names.is_empty() {
        return Ok(());
    }
    // Each requested name is validated independently: a name is only
    // "absent" if it exists in no participant's declared layer catalog.
    // A name valid in one Vault and absent from another is expected, not an
    // error (layer selection applies per participant Vault). Existence comes
    // from each Vault's declared `.hatchdoor-layer` catalog, not from
    // current note counts, so a declared-but-currently-empty layer still
    // validates.
    let known: std::collections::BTreeSet<&str> = snapshots
        .flat_map(|snapshot| snapshot.layer_catalog.iter())
        .map(|info| info.name.as_str())
        .collect();
    let missing: Vec<&String> = names
        .iter()
        .filter(|name| !known.contains(name.as_str()))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(error(
            None,
            "invalid_layer_selection",
            &format!(
                "the requested named layer(s) are absent from every usable Vault: {}",
                missing
                    .iter()
                    .map(|name| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            false,
        ))
    }
}

fn note_for(snapshot: &VaultSnapshotRead, slug: &str) -> Option<NoteSummary> {
    snapshot
        .notes
        .iter()
        .find(|note| note.slug == slug)
        .map(note_summary)
}

fn note_summary(note: &crate::cache::vault_snapshots::VaultSnapshotNote) -> NoteSummary {
    NoteSummary {
        title: note.title.clone(),
        slug: note.slug.clone(),
        relative_path: note.relative_path.clone(),
        layer: note.layer.clone(),
        metadata: note.metadata.clone(),
    }
}

struct ResultDetails {
    chunk_id: i64,
    heading_path: Option<String>,
    content: String,
    score: f32,
}

fn result_for(
    vault_id: VaultId,
    snapshot: &VaultSnapshotRead,
    note: NoteSummary,
    details: ResultDetails,
) -> VaultSearchResult {
    let outbound_links = snapshot
        .links
        .iter()
        .filter(|link| link.source_slug == note.slug)
        .filter_map(|link| {
            snapshot
                .notes
                .iter()
                .find(|target| target.slug == link.target_slug)
                .map(|target| OutboundLink {
                    slug: target.slug.clone(),
                    title: target.title.clone(),
                })
        })
        .collect();
    VaultSearchResult {
        vault_id,
        chunk_id: details.chunk_id,
        note_slug: note.slug,
        note_title: note.title,
        note_path: note.relative_path,
        heading_path: details.heading_path,
        content: details.content,
        score: details.score,
        layer: note.layer,
        outbound_links,
        // The empty `properties` object is the established wire shape, not an
        // omission; see the test that pins it.
        metadata: NoteMetadata {
            tags: note.metadata.tags,
            aliases: note.metadata.aliases,
            properties: serde_json::Value::Object(serde_json::Map::new()),
        },
    }
}

fn layer_matches(selection: &LayerSelection, layer: Option<&str>) -> bool {
    selection.is_all()
        || (layer.is_none() && selection.includes_default())
        || layer.is_some_and(|layer| selection.named_layers().iter().any(|name| name == layer))
}

fn apply_per_note_cap(
    raw: Vec<VaultSearchResult>,
    per_note_cap: usize,
    limit: usize,
) -> Vec<VaultSearchResult> {
    if per_note_cap == 0 || limit == 0 {
        return Vec::new();
    }
    let mut seen = BTreeMap::<(VaultId, String), usize>::new();
    let mut results = Vec::with_capacity(limit.min(raw.len()));
    for result in raw {
        let count = seen
            .entry((result.vault_id, result.note_slug.clone()))
            .or_default();
        if *count >= per_note_cap {
            continue;
        }
        *count += 1;
        results.push(result);
        if results.len() == limit {
            break;
        }
    }
    results
}

fn rank_order(left: &VaultSearchResult, right: &VaultSearchResult) -> Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.vault_id.cmp(&right.vault_id))
        .then_with(|| left.note_slug.cmp(&right.note_slug))
        .then_with(|| left.chunk_id.cmp(&right.chunk_id))
}

fn error(vault_id: Option<VaultId>, code: &str, message: &str, retryable: bool) -> VaultReadError {
    VaultReadError {
        code: code.to_string(),
        message: message.to_string(),
        vault_id,
        retryable,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Condvar, mpsc};
    use std::time::Duration;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::search::{LayerSelection, SearchMode};
    use crate::vault::{NoteMetadata, VaultIndex};
    use crate::vault_read::{VaultParticipantState, VaultScope};
    use crate::vault_registry::{NewVaultDefinition, VaultId, VaultRegistryStore, VaultSource};
    use crate::vault_runtime::VaultCollectionRuntime;

    use super::{VaultSearchCore, VaultSearchRequest, VaultSearchResult};

    #[derive(Default)]
    struct DominantNoteEmbedder {
        calls: AtomicUsize,
    }

    impl DominantNoteEmbedder {
        fn reset_calls(&self) {
            self.calls.store(0, Ordering::SeqCst);
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Embedder for DominantNoteEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(texts
                .iter()
                .map(|text| {
                    if text.contains("alpha") || text == "any query" {
                        vec![0.0; 384]
                    } else {
                        vec![1.0; 384]
                    }
                })
                .collect())
        }

        fn embedding_dim(&self) -> usize {
            384
        }

        fn identity(&self) -> String {
            "dominant-note-384".to_string()
        }

        fn token_count(&self, text: &str, _add_special_tokens: bool) -> Result<usize, String> {
            Ok(text.split_whitespace().count())
        }
    }

    struct Workspace {
        _directory: TempDir,
        cache: SqliteCache,
        vaults: VaultCollectionRuntime,
        vault_ids: Vec<VaultId>,
        vault_paths: Vec<std::path::PathBuf>,
    }

    fn workspace(vaults: &[(&str, &[(&str, &str)])]) -> Workspace {
        workspace_on(vaults, |_| SqliteCache::in_memory(384).expect("cache"))
    }

    /// The file-backed twin of [`workspace`]. Production opens the cache from
    /// a file, and only that source pools and accounts read connections, so an
    /// in-memory workspace cannot observe the reader ceiling at all.
    fn file_backed_workspace(vaults: &[(&str, &[(&str, &str)])]) -> Workspace {
        workspace_on(vaults, |directory| {
            SqliteCache::open(directory.join("cache.sqlite3"), 384).expect("file cache")
        })
    }

    fn workspace_on(
        vaults: &[(&str, &[(&str, &str)])],
        open_cache: impl FnOnce(&Path) -> SqliteCache,
    ) -> Workspace {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let cache = open_cache(directory.path());
        let runtime = VaultCollectionRuntime::new();
        let mut revision = 0;
        let mut registry = None;
        let mut vault_ids = Vec::new();
        let mut paths = Vec::new();
        for (number, (name, files)) in vaults.iter().enumerate() {
            let path = directory.path().join(format!("vault-{number}"));
            write_files(&path, files);
            let next = store
                .add(
                    revision,
                    NewVaultDefinition {
                        name: (*name).to_string(),
                        enabled: true,
                        source: VaultSource::Local { path: path.clone() },
                        exclude_patterns: Vec::new(),
                        https_credentials: None,
                        archive_folder: None,
                        commit_identity: None,
                    },
                )
                .expect("add Vault");
            revision = next.revision();
            vault_ids.push(
                next.definitions()
                    .find(|definition| definition.name() == *name)
                    .expect("new definition")
                    .vault_id(),
            );
            paths.push(path);
            registry = Some(next);
        }
        let registry = registry.unwrap_or_else(|| match store.load().expect("load registry") {
            crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
            crate::vault_registry::VaultRegistryState::Recovery(_) => panic!("unexpected recovery"),
        });
        runtime.reconcile(&store, &registry);
        let embedder = StubEmbedder::new(384);
        for (vault_id, path) in vault_ids.iter().zip(&paths) {
            let index = VaultIndex::build(path).expect("index");
            cache
                .replace_vault_snapshot(*vault_id, &index, &embedder)
                .expect("publish snapshot");
        }
        Workspace {
            _directory: directory,
            cache,
            vaults: runtime,
            vault_ids,
            vault_paths: paths,
        }
    }

    fn write_files(root: &Path, files: &[(&str, &str)]) {
        for (path, contents) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
            std::fs::write(path, contents).expect("write note");
        }
    }

    /// A browser answers one typed query with several overlapping requests,
    /// and the sidebar adds its own. More of those than the cache has reader
    /// slots must queue for a slot, not collect a 503.
    #[test]
    fn concurrent_searches_past_the_reader_ceiling_queue_rather_than_fail() {
        let workspace = file_backed_workspace(&[
            ("First", &[("Home.md", "# Home\n\nzeno needle")]),
            ("Second", &[("Home.md", "# Home\n\nzeno needle")]),
        ]);
        let embedder = StubEmbedder::new(384);
        let core = VaultSearchCore::new(&workspace.cache, &workspace.vaults, &embedder);

        // One search at a time. A slot dropped on the floor would push this
        // above zero and never come back down.
        for round in 0..10 {
            core.search(request(VaultScope::All, "zeno"))
                .expect("sequential search");
            assert_eq!(
                workspace.cache.active_read_leases(),
                0,
                "a search must return its reader slot; {round} rounds in, it had not"
            );
        }

        // Search-as-you-type plus the sidebar's tree and recent calls put
        // well over four reads in flight at once. Each one must come back with
        // its results, so a search that failed for some unrelated reason
        // cannot pass this as though it had been served.
        for concurrency in [2_usize, 4, 6, 8] {
            std::thread::scope(|scope| {
                let searches = (0..concurrency)
                    .map(|_| scope.spawn(|| core.search(request(VaultScope::All, "zeno"))))
                    .collect::<Vec<_>>();
                for search in searches {
                    let hits = search
                        .join()
                        .expect("search thread")
                        .unwrap_or_else(|error| {
                            panic!(
                                "{concurrency} concurrent searches must all be served: {error:?}"
                            )
                        });
                    assert_eq!(
                        hits.data.results.len(),
                        2,
                        "a served search must still return both Vaults' matches"
                    );
                }
            });
        }
    }

    /// Every production embedder runs inference behind its own `Mutex`, so
    /// concurrent semantic searches queue on it one at a time. A search that
    /// took its reader slot first would pin that slot for the whole wait while
    /// touching no database at all, and four typed-ahead searches were enough
    /// to starve an unrelated `/tree` or `/recent` read.
    #[test]
    fn a_search_holds_no_reader_slot_while_the_embedder_serializes() {
        let workspace =
            file_backed_workspace(&[("First", &[("Home.md", "# Home\n\nzeno needle")])]);
        let (arrived_tx, arrived_rx) = mpsc::channel();
        let release = Arc::new((std::sync::Mutex::new(false), Condvar::new()));
        let embedder = SerializedSlowEmbedder {
            inner: StubEmbedder::new(384),
            model: std::sync::Mutex::new(()),
            arrived: arrived_tx,
            release: release.clone(),
        };
        let core = VaultSearchCore::new(&workspace.cache, &workspace.vaults, &embedder);

        std::thread::scope(|scope| {
            let searches = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        let mut semantic = request(VaultScope::All, "zeno");
                        semantic.mode = SearchMode::Semantic;
                        core.search(semantic)
                    })
                })
                .collect::<Vec<_>>();

            // Hold every search inside the embedder at once, the state four
            // typed-ahead queries reach in production. A search that never
            // arrives fails here rather than hanging the test.
            for index in 0..4 {
                arrived_rx
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap_or_else(|_| panic!("search {index} never reached the embedder"));
            }
            // The lease count is the assertion that isolates this from the
            // reader queue: with a slot held across inference the sidebar read
            // would merely be delayed until one search finished, and a test
            // that only checked `is_ok` would pass on the queue alone.
            let held = workspace.cache.active_read_leases();
            let sidebar = workspace
                .cache
                .snapshot_note_content(workspace.vault_ids[0], "home");

            let (released, wake) = &*release;
            *released
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
            wake.notify_all();

            for search in searches {
                search
                    .join()
                    .expect("search thread")
                    .expect("a search held behind the embedder must still complete");
            }
            assert_eq!(
                held, 0,
                "four searches inside the embedder must hold no reader slot between them"
            );
            assert!(
                sidebar.is_ok(),
                "an unrelated read was starved by searches waiting on the embedder: {:?}",
                sidebar.err()
            );
        });
    }

    /// Stands in for the production embedders, which all wrap inference in a
    /// `Mutex` (see `fastembed_embedder.rs`). Announces each arrival and then
    /// parks until the test releases it, so every caller can be held inside
    /// inference at once.
    struct SerializedSlowEmbedder {
        inner: StubEmbedder,
        model: std::sync::Mutex<()>,
        arrived: mpsc::Sender<()>,
        release: Arc<(std::sync::Mutex<bool>, Condvar)>,
    }

    impl Embedder for SerializedSlowEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            self.arrived.send(()).expect("announce arrival");
            let (released, wake) = &*self.release;
            let mut parked = released
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // Bounded so a failing assertion ends the test rather than
            // leaving these threads parked for the scope to join forever.
            while !*parked {
                let (next, wait) = wake
                    .wait_timeout(parked, Duration::from_secs(5))
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                parked = next;
                if wait.timed_out() {
                    break;
                }
            }
            drop(parked);
            let _inference = self
                .model
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
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

    fn request(scope: VaultScope, query: &str) -> VaultSearchRequest {
        VaultSearchRequest {
            scope,
            query: query.to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 1,
            layers: LayerSelection::default_surface(),
        }
    }

    /// A structure-only Vault has chunk text but no vectors. Semantic search
    /// must name it rather than let the per-chunk vector check drop it in
    /// silence, which a caller reads as "this Vault has no matches" instead of
    /// "this Vault has not been embedded yet".
    #[test]
    fn semantic_search_reports_a_vectorless_vault_instead_of_dropping_it_silently() {
        let workspace = workspace(&[("Alpha", &[("Home.md", "# Home\n\nneedle body")])]);
        let embedder = StubEmbedder::new(384);
        let vault_id = workspace.vault_ids[0];
        // Re-publish as structure-only, the state between a first index's two
        // passes.
        workspace
            .cache
            .disconnect_vault_snapshot(vault_id)
            .expect("drop the searchable generation");
        workspace
            .cache
            .publish_vault_structure_snapshot(
                vault_id,
                &VaultIndex::build(&workspace.vault_paths[0]).expect("index"),
                &embedder,
                true,
            )
            .expect("publish structure-only snapshot");

        let core = VaultSearchCore::new(&workspace.cache, &workspace.vaults, &embedder);
        let semantic = core
            .search(VaultSearchRequest {
                mode: SearchMode::Semantic,
                ..request(VaultScope::All, "needle")
            })
            .expect("semantic search");

        assert_eq!(
            semantic.participants[0].state,
            VaultParticipantState::NotSearchable
        );
        assert!(
            semantic.partial,
            "a projection missing a Vault's semantic contribution is partial"
        );

        // Keyword search reads the same structural rows and is unaffected.
        let keyword = core
            .search(request(VaultScope::All, "needle"))
            .expect("keyword search");
        assert_eq!(keyword.participants[0].state, VaultParticipantState::Fresh);
        assert!(!keyword.partial);
        assert_eq!(keyword.data.results.len(), 1);
    }

    #[test]
    fn global_keyword_search_keeps_duplicate_slugs_vault_qualified_and_honours_one_scope() {
        let workspace = workspace(&[
            ("First", &[("Home.md", "# Home\n\nshared term")]),
            ("Second", &[("Home.md", "# Home\n\nshared term")]),
        ]);
        let embedder = StubEmbedder::new(384);
        let core = VaultSearchCore::new(&workspace.cache, &workspace.vaults, &embedder);

        let all = core
            .search(request(VaultScope::All, "shared"))
            .expect("all search");
        assert_eq!(all.data.results.len(), 2);
        let mut expected = workspace.vault_ids.clone();
        expected.sort();
        assert_eq!(
            all.data
                .results
                .iter()
                .map(|result| result.vault_id)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(
            all.data
                .results
                .iter()
                .all(|result| result.note_slug == "home")
        );

        let one = core
            .search(request(VaultScope::One(workspace.vault_ids[1]), "shared"))
            .expect("one search");
        assert_eq!(one.data.results.len(), 1);
        assert_eq!(one.data.results[0].vault_id, workspace.vault_ids[1]);
    }

    #[test]
    fn semantic_search_overfetches_to_fill_requested_distinct_notes() {
        // Alpha's chunks rank ahead of Bravo's deterministic vector. More than
        // the former fixed raw window (limit * 4 = 8) therefore contains only
        // Alpha and the per-note cap leaves the caller short.
        // The VaultSearchCore seam must backfill enough candidates to surface
        // Bravo as the requested second distinct note.
        let alpha = format!("# Alpha\n\n{}", "alpha ".repeat(9_000));
        let workspace = workspace(&[(
            "Only",
            &[("alpha.md", &alpha), ("bravo.md", "# Bravo\n\nbravo")],
        )]);
        let embedder = DominantNoteEmbedder::default();
        workspace
            .cache
            .replace_vault_snapshot(
                workspace.vault_ids[0],
                &VaultIndex::build(&workspace.vault_paths[0]).expect("index"),
                &embedder,
            )
            .expect("publish deterministic vectors");
        embedder.reset_calls();
        let alpha_chunks: i64 = workspace
            .cache
            .connection()
            .expect("connection")
            .query_row(
                "SELECT COUNT(*) FROM vault_chunks WHERE note_slug = 'alpha'",
                [],
                |row| row.get(0),
            )
            .expect("count alpha chunks");
        assert!(
            alpha_chunks > 8,
            "fixture must exceed the former fixed raw candidate window"
        );
        let core = VaultSearchCore::new(&workspace.cache, &workspace.vaults, &embedder);

        let response = core
            .search(VaultSearchRequest {
                mode: SearchMode::Semantic,
                limit: 2,
                per_note_cap: 1,
                ..request(VaultScope::One(workspace.vault_ids[0]), "any query")
            })
            .expect("semantic search");

        assert_eq!(
            response
                .data
                .results
                .iter()
                .map(|result| result.note_slug.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "bravo"]
        );
        assert_eq!(
            embedder.calls(),
            1,
            "one semantic request must embed its query once across progressive KNN windows"
        );
    }

    #[test]
    fn semantic_candidate_ceiling_returns_the_best_available_partial_set() {
        // This isolates bounded candidate selection from the KNN implementation:
        // 201 Alpha chunks precede Bravo, but 200 is the explicit ceiling.
        let vault_id =
            VaultId::from_str("018f47a0-7768-4d0c-8da3-5aa28d1c31c7").expect("known Vault ID");
        let result = |note_slug: &str, chunk_id| VaultSearchResult {
            vault_id,
            chunk_id,
            note_slug: note_slug.to_string(),
            note_title: note_slug.to_string(),
            note_path: format!("{note_slug}.md"),
            heading_path: None,
            content: String::new(),
            score: 1.0,
            layer: None,
            outbound_links: Vec::new(),
            metadata: NoteMetadata::default(),
        };
        let mut candidates = (0..201)
            .map(|chunk_id| result("alpha", chunk_id))
            .collect::<Vec<_>>();
        candidates.push(result("bravo", 201));
        let mut requested_depths = Vec::new();

        let results = super::progressively_cap_semantic_results(2, 1, |raw_k| {
            requested_depths.push(raw_k);
            Ok(candidates[..raw_k.min(candidates.len())].to_vec())
        })
        .expect("bounded selection");

        assert_eq!(requested_depths, vec![8, 16, 32, 64, 128, 200]);
        assert_eq!(
            results
                .iter()
                .map(|result| result.note_slug.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha"]
        );
    }

    #[test]
    fn keyword_search_keeps_metadata_and_hits_in_one_published_generation() {
        let workspace = workspace(&[("Only", &[("Home.md", "# Old title\n\nneedle old")])]);
        let vault_id = workspace.vault_ids[0];
        let cache = SqliteCache::open(workspace._directory.path().join("concurrent.sqlite3"), 384)
            .expect("file-backed cache");
        cache
            .replace_vault_snapshot(
                vault_id,
                &VaultIndex::build(&workspace.vault_paths[0]).expect("old index"),
                &StubEmbedder::new(384),
            )
            .expect("publish old generation");
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let hook = Arc::new({
            let entered = entered.clone();
            let release = release.clone();
            move || {
                entered.wait();
                release.wait();
            }
        });

        std::thread::scope(|scope| {
            let search = scope.spawn(|| {
                let embedder = StubEmbedder::new(384);
                VaultSearchCore::new(&cache, &workspace.vaults, &embedder)
                    .with_after_snapshot_read_hook(hook)
                    .search(request(VaultScope::One(vault_id), "needle"))
                    .expect("search")
            });
            entered.wait();
            write_files(
                &workspace.vault_paths[0],
                &[("Home.md", "# New title\n\nneedle new")],
            );
            cache
                .replace_vault_snapshot(
                    vault_id,
                    &VaultIndex::build(&workspace.vault_paths[0]).expect("new index"),
                    &StubEmbedder::new(384),
                )
                .expect("publish new generation");
            release.wait();
            let response = search.join().expect("search thread");
            assert_eq!(response.data.results.len(), 1);
            assert_eq!(response.data.results[0].note_title, "Home");
            assert!(response.data.results[0].content.contains("needle old"));
        });
    }

    #[test]
    fn named_layers_are_independent_per_vault_and_absent_everywhere_is_an_error() {
        let workspace = workspace(&[
            (
                "Layered",
                &[
                    ("sources/.hatchdoor-layer", "sources"),
                    ("sources/Clipping.md", "# Clipping\n\nneedle"),
                ],
            ),
            ("Plain", &[("Home.md", "# Home\n\nneedle")]),
        ]);
        let embedder = StubEmbedder::new(384);
        let core = VaultSearchCore::new(&workspace.cache, &workspace.vaults, &embedder);
        let (sources, warnings) =
            LayerSelection::parse(&["sources".to_string()], &["sources".to_string()]);
        assert!(warnings.is_empty());

        let selected = core
            .search(VaultSearchRequest {
                layers: sources,
                ..request(VaultScope::All, "needle")
            })
            .expect("selected layer search");
        assert_eq!(selected.data.results.len(), 1);
        assert_eq!(selected.data.results[0].vault_id, workspace.vault_ids[0]);
        assert!(
            selected
                .participants
                .iter()
                .all(|participant| participant.state == VaultParticipantState::Fresh)
        );

        let (missing, _) =
            LayerSelection::parse(&["missing".to_string()], &["missing".to_string()]);
        let error = core
            .search(VaultSearchRequest {
                layers: missing,
                ..request(VaultScope::All, "needle")
            })
            .expect_err("globally absent named layer must fail");
        assert_eq!(error.code, "invalid_layer_selection");
    }

    #[test]
    fn one_absent_named_layer_fails_even_when_another_requested_name_exists() {
        // Regression for #93: validation must check each requested name
        // independently. A single existential OR across all requested names
        // and all notes let `sources,ghost` succeed whenever `sources` alone
        // existed anywhere, silently swallowing the absent `ghost` layer.
        let workspace = workspace(&[
            (
                "Layered",
                &[
                    ("sources/.hatchdoor-layer", "sources"),
                    ("sources/Clipping.md", "# Clipping\n\nneedle"),
                ],
            ),
            ("Plain", &[("Home.md", "# Home\n\nneedle")]),
        ]);
        let embedder = StubEmbedder::new(384);
        let core = VaultSearchCore::new(&workspace.cache, &workspace.vaults, &embedder);
        let (selection, warnings) = LayerSelection::parse(
            &["sources".to_string(), "ghost".to_string()],
            &["sources".to_string(), "ghost".to_string()],
        );
        assert!(warnings.is_empty());

        let error = core
            .search(VaultSearchRequest {
                layers: selection,
                ..request(VaultScope::All, "needle")
            })
            .expect_err("a request naming one existent and one globally absent layer must fail");
        assert_eq!(error.code, "invalid_layer_selection");
    }

    #[test]
    fn a_declared_layer_with_currently_no_notes_is_a_valid_selection() {
        // Regression for #93: layer existence must come from the declared
        // `.hatchdoor-layer` catalog, not be inferred from note counts. A
        // layer marker with zero notes classified under it today must still
        // validate, not be wrongly rejected as nonexistent.
        let workspace = workspace(&[(
            "Layered",
            &[
                ("archive/.hatchdoor-layer", "archive"),
                ("Home.md", "# Home\n\nneedle"),
            ],
        )]);
        let embedder = StubEmbedder::new(384);
        let core = VaultSearchCore::new(&workspace.cache, &workspace.vaults, &embedder);
        let (selection, warnings) =
            LayerSelection::parse(&["archive".to_string()], &["archive".to_string()]);
        assert!(warnings.is_empty());

        let response = core
            .search(VaultSearchRequest {
                layers: selection,
                ..request(VaultScope::All, "needle")
            })
            .expect("a declared but currently empty layer must be a valid selection");
        assert!(response.data.results.is_empty());
    }

    #[test]
    fn semantic_search_ranks_one_global_window_and_stale_participants_keep_their_rank() {
        let workspace = workspace(&[
            ("First", &[("Home.md", "# Home\n\nsemantic needle")]),
            ("Second", &[("Home.md", "# Home\n\nsemantic needle")]),
        ]);
        let stale_index = VaultIndex::build(&workspace.vault_paths[1]).expect("stale index");
        std::fs::remove_file(workspace.vault_paths[1].join("Home.md")).expect("break reindex");
        workspace
            .cache
            .replace_vault_snapshot(
                workspace.vault_ids[1],
                &stale_index,
                &StubEmbedder::new(384),
            )
            .expect_err("failed refresh keeps the prior snapshot stale");

        let embedder = StubEmbedder::new(384);
        let core = VaultSearchCore::new(&workspace.cache, &workspace.vaults, &embedder);
        let response = core
            .search(VaultSearchRequest {
                mode: SearchMode::Semantic,
                ..request(VaultScope::All, "semantic needle")
            })
            .expect("semantic search");
        assert_eq!(response.data.results.len(), 2);
        assert!(response.partial);
        assert!(
            response
                .participants
                .iter()
                .any(|participant| participant.vault_id == workspace.vault_ids[1]
                    && participant.state == VaultParticipantState::Stale)
        );
        assert!(
            response
                .data
                .results
                .iter()
                .any(|result| result.vault_id == workspace.vault_ids[1])
        );
    }

    /// Search results have never carried a note's frontmatter properties: the
    /// projection was always handed an empty selection, so `properties` has
    /// always serialized as an empty object. That empty object is part of the
    /// wire shape, so it must survive the projection's removal rather than
    /// becoming a missing field, a null, or the note's actual properties.
    ///
    /// Every mode is checked even though one function shapes them all: that
    /// sharing is exactly what a later change could quietly undo.
    #[test]
    fn a_search_result_serializes_its_metadata_with_an_empty_properties_object() {
        let workspace = workspace(&[(
            "Only",
            &[(
                "Home.md",
                "---\ntags: [topic/sub]\naliases: [Homepage]\nstatus: draft\n---\n# Home\n\nneedle body",
            )],
        )]);
        let embedder = StubEmbedder::new(384);
        let core = VaultSearchCore::new(&workspace.cache, &workspace.vaults, &embedder);
        let expected = serde_json::json!({
            "tags": ["topic/sub"],
            "aliases": ["Homepage"],
            "properties": {},
        });

        for (mode, query) in [
            (SearchMode::Keyword, "needle"),
            (SearchMode::Semantic, "needle"),
            // A tag query answers from structural rows rather than chunks, so
            // it reaches the shared result shaping by its own route.
            (SearchMode::Semantic, "#topic"),
        ] {
            let response = core
                .search(VaultSearchRequest {
                    mode,
                    ..request(VaultScope::All, query)
                })
                .unwrap_or_else(|error| panic!("search {query:?}: {error:?}"));

            assert_eq!(response.data.results.len(), 1, "search {query:?}");
            let serialized =
                serde_json::to_value(&response.data.results[0]).expect("serialize result");
            assert_eq!(
                serialized["metadata"], expected,
                "search {query:?} keeps its tags and aliases and an empty properties object"
            );
        }
    }

    #[test]
    fn tag_shorthand_is_vault_qualified_path_ordered_and_ignores_the_chunk_cap() {
        let workspace = workspace(&[
            (
                "First",
                &[("zeta.md", "---\ntags: [topic/sub]\n---\n# Zeta")],
            ),
            (
                "Second",
                &[("alpha.md", "---\ntags: [topic/sub]\n---\n# Alpha")],
            ),
        ]);
        let embedder = StubEmbedder::new(384);
        let core = VaultSearchCore::new(&workspace.cache, &workspace.vaults, &embedder);
        let response = core
            .search(VaultSearchRequest {
                per_note_cap: 0,
                ..request(VaultScope::All, "#topic")
            })
            .expect("tag shorthand");
        assert_eq!(response.data.results.len(), 2);
        assert_eq!(response.data.results[0].note_path, "alpha");
        assert_eq!(response.data.results[1].note_path, "zeta");
        assert!(
            response
                .data
                .results
                .iter()
                .all(|result| result.chunk_id == 0)
        );
    }
}
