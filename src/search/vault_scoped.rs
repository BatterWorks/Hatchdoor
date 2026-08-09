//! Vault-qualified search over the published disposable cache snapshots.

use std::cmp::Ordering;
use std::collections::BTreeMap;

#[cfg(test)]
use std::sync::Arc;

use serde::Serialize;

use crate::cache::{SqliteCache, vault_snapshots::VaultSnapshotRead};
use crate::embed::Embedder;
use crate::vault::NoteSummary;
use crate::vault_read::{
    VaultParticipant, VaultParticipantState, VaultReadError, VaultReadProjection, VaultScope,
    selected_vaults,
};
use crate::vault_registry::VaultId;
use crate::vault_runtime::VaultCollectionRuntime;

use super::{LayerSelection, NoteFilters, OutboundLink, SearchMode, tag_prefix_query};

#[cfg(test)]
type SnapshotReadHook = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug, Clone)]
pub struct VaultSearchRequest {
    pub scope: VaultScope,
    pub query: String,
    pub mode: SearchMode,
    pub limit: usize,
    pub per_note_cap: usize,
    pub filters: NoteFilters,
    pub include_properties: Vec<String>,
    pub layers: LayerSelection,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
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
            #[cfg(test)]
            after_snapshot_read_hook: None,
        }
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
        let mut cache_snapshot = self
            .cache
            .read_snapshot()
            .map_err(|message| error(None, "search_unavailable", &message, true))?;
        let mut snapshots = BTreeMap::new();
        let mut participants = Vec::with_capacity(selected.len());
        for vault in selected {
            match SqliteCache::read_vault_snapshot_on(&cache_snapshot, vault.vault_id) {
                Ok(Some(snapshot)) => {
                    let state = match snapshot.status.freshness {
                        crate::cache::vault_snapshots::VaultSnapshotFreshness::Fresh => {
                            VaultParticipantState::Fresh
                        }
                        crate::cache::vault_snapshots::VaultSnapshotFreshness::Stale => {
                            VaultParticipantState::Stale
                        }
                    };
                    participants.push(VaultParticipant {
                        vault_id: vault.vault_id,
                        vault_name: vault.vault_name,
                        state,
                        error: None,
                    });
                    snapshots.insert(vault.vault_id, snapshot.read);
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
        let results = if let Some(tag) = tag_prefix_query(&request.query) {
            tag_results(&request, &snapshots, &tag)
        } else {
            let raw = match request.mode {
                SearchMode::Semantic => semantic_hits(
                    &request,
                    self.cache,
                    &cache_snapshot,
                    self.embedder,
                    &snapshots,
                )?,
                SearchMode::Keyword => {
                    keyword_hits(&request, self.cache, &cache_snapshot, &snapshots)?
                }
            };
            apply_per_note_cap(raw, request.per_note_cap, request.limit)
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

fn semantic_hits(
    request: &VaultSearchRequest,
    cache: &SqliteCache,
    conn: &rusqlite::Connection,
    embedder: &dyn Embedder,
    snapshots: &BTreeMap<VaultId, VaultSnapshotRead>,
) -> Result<Vec<VaultSearchResult>, VaultReadError> {
    let raw_k = request.limit.saturating_mul(request.per_note_cap).min(200);
    if raw_k == 0 {
        return Ok(Vec::new());
    }
    if request.filters.is_empty() {
        let ids = snapshots.keys().copied().collect::<Vec<_>>();
        let hits = cache
            .vault_semantic_search_layered(
                conn,
                &ids,
                embedder,
                &request.query,
                raw_k,
                &request.layers,
            )
            .map_err(|message| error(None, "search_unavailable", &message, true))?;
        let results = hits
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
                    &request.include_properties,
                ))
            })
            .collect::<Vec<_>>();
        return Ok(results);
    }
    let vector = embedder
        .embed(&[format!("{}{}", embedder.query_prefix(), request.query)])
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
        })?;
    let mut hits = Vec::new();
    for (vault_id, snapshot) in snapshots {
        for chunk in &snapshot.chunks {
            if !layer_matches(&request.layers, chunk.layer.as_deref()) {
                continue;
            }
            let Some(note) = note_for(snapshot, &chunk.note_slug) else {
                continue;
            };
            if !request.filters.matches(&note) {
                continue;
            }
            let distance = euclidean_distance(&vector, &chunk.embedding)
                .map_err(|message| error(Some(*vault_id), "search_unavailable", &message, true))?;
            hits.push(result_for(
                *vault_id,
                snapshot,
                note,
                ResultDetails {
                    chunk_id: chunk.chunk_id,
                    heading_path: chunk.heading_path.clone(),
                    content: chunk.content.clone(),
                    score: (1.0 - distance).clamp(0.0, 1.0),
                },
                &request.include_properties,
            ));
        }
    }
    hits.sort_by(rank_order);
    Ok(hits)
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
        if !request.filters.matches(&note) {
            continue;
        }
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
            &request.include_properties,
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
            if !layer_matches(&request.layers, note.layer.as_deref())
                || !request.filters.matches(&summary)
            {
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
                &request.include_properties,
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
    include_properties: &[String],
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
        metadata: super::project_metadata(&note.metadata, include_properties),
    }
}

fn layer_matches(selection: &LayerSelection, layer: Option<&str>) -> bool {
    selection.is_all()
        || (layer.is_none() && selection.includes_default())
        || layer.is_some_and(|layer| selection.named_layers().iter().any(|name| name == layer))
}

fn euclidean_distance(query: &[f32], embedding: &[u8]) -> Result<f32, String> {
    if embedding.len() != std::mem::size_of_val(query) {
        return Err("cached embedding dimension mismatch".to_string());
    }
    Ok(embedding
        .chunks_exact(4)
        .zip(query)
        .map(|(bytes, value)| {
            let candidate = f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let difference = candidate - value;
            difference * difference
        })
        .sum::<f32>()
        .sqrt())
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
    use std::sync::{Arc, Barrier};

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::StubEmbedder;
    use crate::search::{LayerSelection, NoteFilters, SearchMode};
    use crate::vault::VaultIndex;
    use crate::vault_read::{VaultParticipantState, VaultScope};
    use crate::vault_registry::{NewVaultDefinition, VaultId, VaultRegistryStore, VaultSource};
    use crate::vault_runtime::VaultCollectionRuntime;

    use super::{VaultSearchCore, VaultSearchRequest};

    struct Workspace {
        _directory: TempDir,
        cache: SqliteCache,
        vaults: VaultCollectionRuntime,
        vault_ids: Vec<VaultId>,
        vault_paths: Vec<std::path::PathBuf>,
    }

    fn workspace(vaults: &[(&str, &[(&str, &str)])]) -> Workspace {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = VaultRegistryStore::new(directory.path().join("state/vaults.json"));
        let cache = SqliteCache::in_memory(384).expect("cache");
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

    fn request(scope: VaultScope, query: &str) -> VaultSearchRequest {
        VaultSearchRequest {
            scope,
            query: query.to_string(),
            mode: SearchMode::Keyword,
            limit: 10,
            per_note_cap: 1,
            filters: NoteFilters::default(),
            include_properties: Vec::new(),
            layers: LayerSelection::default_surface(),
        }
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
