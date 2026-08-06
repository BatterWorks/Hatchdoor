//! Shared-core, Vault-qualified reads over authoritative Markdown and the
//! disposable shared SQLite snapshot cache.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::cache::{SqliteCache, vault_snapshots::VaultSnapshotRead};
use crate::vault::{Note, NoteLink, NoteLinks};
use crate::vault_registry::VaultId;
use crate::vault_runtime::VaultCollectionRuntime;

/// An explicit collection read target. There is deliberately no selected,
/// default, or sole-Vault variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultScope {
    One(VaultId),
    All,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultReadError {
    pub code: String,
    pub message: String,
    pub vault_id: Option<VaultId>,
    pub retryable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultParticipantState {
    Fresh,
    Stale,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultParticipant {
    pub vault_id: VaultId,
    pub vault_name: String,
    pub state: VaultParticipantState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<VaultReadError>,
}

/// The common one-or-all read envelope future HTTP and MCP adapters share.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultReadProjection<T> {
    pub scope: VaultScope,
    pub collection_revision: u64,
    pub partial: bool,
    pub participants: Vec<VaultParticipant>,
    pub data: T,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultQualifiedNote {
    pub vault_id: VaultId,
    pub note: Note,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultQualifiedLink {
    pub vault_id: VaultId,
    pub link: NoteLink,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultQualifiedLinks {
    pub vault_id: VaultId,
    pub outgoing: Vec<VaultQualifiedLink>,
    pub backlinks: Vec<VaultQualifiedLink>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResolvedVaultNote {
    pub vault_id: VaultId,
    pub slug: String,
    pub relative_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultTree {
    pub vault_id: VaultId,
    pub vault_name: String,
    pub tree: VaultExplorerFolder,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultExplorerFolder {
    pub name: String,
    pub folders: Vec<VaultExplorerFolder>,
    pub notes: Vec<VaultExplorerNote>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultExplorerNote {
    pub vault_id: VaultId,
    pub title: String,
    pub slug: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultStatistics {
    pub vault_id: VaultId,
    pub vault_name: String,
    pub note_count: usize,
    pub tag_count: usize,
    pub link_count: usize,
    pub vault_size_bytes: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultGraph {
    pub vault_id: VaultId,
    pub vault_name: String,
    pub nodes: Vec<VaultGraphNode>,
    pub edges: Vec<VaultGraphEdge>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultGraphNode {
    pub vault_id: VaultId,
    pub slug: String,
    pub title: String,
    pub primary_tag: Option<String>,
    pub backlink_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultGraphEdge {
    pub vault_id: VaultId,
    pub source_slug: String,
    pub target_slug: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VaultRecentNote {
    pub vault_id: VaultId,
    pub title: String,
    pub slug: String,
    pub relative_path: String,
    pub mtime_ns: i64,
}

/// The shared-core facade. Exact reads use the targeted Vault's current
/// Markdown directory; collection projections use only already-published
/// cache snapshots and report their freshness honestly.
pub struct VaultReadCore<'a> {
    cache: &'a SqliteCache,
    vaults: &'a VaultCollectionRuntime,
}

impl<'a> VaultReadCore<'a> {
    pub fn new(cache: &'a SqliteCache, vaults: &'a VaultCollectionRuntime) -> Self {
        Self { cache, vaults }
    }

    pub fn exact_note(
        &self,
        vault_id: VaultId,
        slug: &str,
    ) -> Result<Option<VaultQualifiedNote>, VaultReadError> {
        let index = self.authoritative_index(vault_id)?;
        index
            .read_note_by_slug(slug)
            .map(|note| note.map(|note| VaultQualifiedNote { vault_id, note }))
            .map_err(|error| {
                unavailable(vault_id, "vault_read_unavailable", error.to_string(), true)
            })
    }

    pub fn exact_note_links(
        &self,
        vault_id: VaultId,
        slug: &str,
    ) -> Result<Option<VaultQualifiedLinks>, VaultReadError> {
        let index = self.authoritative_index(vault_id)?;
        Ok(index
            .note_links(slug)
            .map(|links| qualify_links(vault_id, links)))
    }

    pub fn resolve_wikilink(
        &self,
        vault_id: VaultId,
        raw_target: &str,
    ) -> Result<Option<ResolvedVaultNote>, VaultReadError> {
        let index = self.authoritative_index(vault_id)?;
        Ok(index
            .resolve_wikilink(raw_target)
            .map(|note| ResolvedVaultNote {
                vault_id,
                slug: note.slug.clone(),
                relative_path: note.relative_path.clone(),
            }))
    }

    pub fn trees(
        &self,
        scope: VaultScope,
    ) -> Result<VaultReadProjection<Vec<VaultTree>>, VaultReadError> {
        self.collection(scope, |vault_id, vault_name, snapshot| VaultTree {
            vault_id,
            vault_name: vault_name.to_string(),
            tree: tree_for(vault_id, snapshot),
        })
    }

    pub fn statistics(
        &self,
        scope: VaultScope,
    ) -> Result<VaultReadProjection<Vec<VaultStatistics>>, VaultReadError> {
        self.collection(scope, |vault_id, vault_name, snapshot| VaultStatistics {
            vault_id,
            vault_name: vault_name.to_string(),
            note_count: snapshot.notes.len(),
            tag_count: snapshot
                .tags_by_note
                .values()
                .flatten()
                .collect::<BTreeSet<_>>()
                .len(),
            link_count: snapshot.links.len(),
            vault_size_bytes: snapshot.notes.iter().map(|note| note.size_bytes).sum(),
        })
    }

    pub fn graphs(
        &self,
        scope: VaultScope,
    ) -> Result<VaultReadProjection<Vec<VaultGraph>>, VaultReadError> {
        self.collection(scope, |vault_id, vault_name, snapshot| VaultGraph {
            vault_id,
            vault_name: vault_name.to_string(),
            nodes: graph_nodes(vault_id, snapshot),
            edges: snapshot
                .links
                .iter()
                .map(|link| VaultGraphEdge {
                    vault_id,
                    source_slug: link.source_slug.clone(),
                    target_slug: link.target_slug.clone(),
                })
                .collect(),
        })
    }

    pub fn recently_modified(
        &self,
        scope: VaultScope,
        limit: usize,
    ) -> Result<VaultReadProjection<Vec<VaultRecentNote>>, VaultReadError> {
        let projection = self.collection(scope, |vault_id, _vault_name, snapshot| {
            snapshot
                .notes
                .iter()
                .map(|note| VaultRecentNote {
                    vault_id,
                    title: note.title.clone(),
                    slug: note.slug.clone(),
                    relative_path: note.relative_path.clone(),
                    mtime_ns: note.mtime_ns,
                })
                .collect::<Vec<_>>()
        })?;
        let mut notes = projection.data.into_iter().flatten().collect::<Vec<_>>();
        notes.sort_by(|left, right| {
            right
                .mtime_ns
                .cmp(&left.mtime_ns)
                .then_with(|| left.vault_id.cmp(&right.vault_id))
                .then_with(|| left.slug.cmp(&right.slug))
        });
        notes.truncate(limit);
        Ok(VaultReadProjection {
            scope: projection.scope,
            collection_revision: projection.collection_revision,
            partial: projection.partial,
            participants: projection.participants,
            data: notes,
        })
    }

    fn authoritative_index(
        &self,
        vault_id: VaultId,
    ) -> Result<crate::vault::VaultIndex, VaultReadError> {
        let snapshot = self.vaults.snapshot();
        let Some(vault) = snapshot.vaults.get(&vault_id) else {
            return Err(unavailable(
                vault_id,
                "vault_not_found",
                "Vault definition was not found".to_string(),
                false,
            ));
        };
        if !vault.enabled {
            return Err(unavailable(
                vault_id,
                "vault_disabled",
                "Vault is disabled and cannot serve reads".to_string(),
                false,
            ));
        }
        let Some(runtime) = self.vaults.runtime(vault_id) else {
            return Err(unavailable(
                vault_id,
                "vault_unavailable",
                "Vault has no active runtime".to_string(),
                true,
            ));
        };
        runtime
            .authoritative_index()
            .map_err(|error| VaultReadError {
                code: error.code,
                message: error.message,
                vault_id: Some(vault_id),
                retryable: error.retryable,
            })
    }

    fn collection<T>(
        &self,
        scope: VaultScope,
        map: impl Fn(VaultId, &str, &VaultSnapshotRead) -> T,
    ) -> Result<VaultReadProjection<Vec<T>>, VaultReadError> {
        let snapshot = self.vaults.snapshot();
        let selected = selected_vaults(&snapshot, scope)?;
        let mut data = Vec::new();
        let mut participants = Vec::with_capacity(selected.len());
        for selected in selected {
            let published = self.cache.read_vault_snapshot(selected.vault_id);
            let participant = match published {
                Ok(Some(published)) => {
                    let state = match published.status.freshness {
                        crate::cache::vault_snapshots::VaultSnapshotFreshness::Fresh => {
                            VaultParticipantState::Fresh
                        }
                        crate::cache::vault_snapshots::VaultSnapshotFreshness::Stale => {
                            VaultParticipantState::Stale
                        }
                    };
                    data.push(map(
                        selected.vault_id,
                        &selected.vault_name,
                        &published.read,
                    ));
                    VaultParticipant {
                        vault_id: selected.vault_id,
                        vault_name: selected.vault_name,
                        state,
                        error: None,
                    }
                }
                Ok(None) => unavailable_participant(
                    selected.vault_id,
                    selected.vault_name,
                    "Vault has no participating searchable snapshot".to_string(),
                ),
                Err(message) => {
                    unavailable_participant(selected.vault_id, selected.vault_name, message)
                }
            };
            if matches!(scope, VaultScope::One(_))
                && participant.state == VaultParticipantState::Unavailable
            {
                return Err(participant
                    .error
                    .expect("unavailable participant carries an error"));
            }
            participants.push(participant);
        }
        Ok(VaultReadProjection {
            scope,
            collection_revision: snapshot.collection_revision,
            partial: participants
                .iter()
                .any(|participant| participant.state != VaultParticipantState::Fresh),
            participants,
            data,
        })
    }
}

pub(crate) struct SelectedVault {
    pub(crate) vault_id: VaultId,
    pub(crate) vault_name: String,
}

pub(crate) fn selected_vaults(
    snapshot: &crate::vault_runtime::VaultCollectionSnapshot,
    scope: VaultScope,
) -> Result<Vec<SelectedVault>, VaultReadError> {
    let selected = match scope {
        VaultScope::All => snapshot
            .vaults
            .values()
            .filter(|vault| vault.enabled)
            .collect::<Vec<_>>(),
        VaultScope::One(vault_id) => {
            let Some(vault) = snapshot.vaults.get(&vault_id) else {
                return Err(unavailable(
                    vault_id,
                    "vault_not_found",
                    "Vault definition was not found".to_string(),
                    false,
                ));
            };
            if !vault.enabled {
                return Err(unavailable(
                    vault_id,
                    "vault_disabled",
                    "Vault is disabled and does not participate in collection reads".to_string(),
                    false,
                ));
            }
            vec![vault]
        }
    };

    Ok(selected
        .into_iter()
        .map(|vault| SelectedVault {
            vault_id: vault.vault_id,
            vault_name: vault.name.clone(),
        })
        .collect())
}

fn unavailable_participant(
    vault_id: VaultId,
    vault_name: String,
    message: String,
) -> VaultParticipant {
    VaultParticipant {
        vault_id,
        vault_name,
        state: VaultParticipantState::Unavailable,
        error: Some(unavailable(vault_id, "vault_unavailable", message, true)),
    }
}

fn unavailable(vault_id: VaultId, code: &str, message: String, retryable: bool) -> VaultReadError {
    VaultReadError {
        code: code.to_string(),
        message,
        vault_id: Some(vault_id),
        retryable,
    }
}

fn qualify_links(vault_id: VaultId, links: NoteLinks) -> VaultQualifiedLinks {
    VaultQualifiedLinks {
        vault_id,
        outgoing: links
            .outgoing
            .into_iter()
            .map(|link| VaultQualifiedLink { vault_id, link })
            .collect(),
        backlinks: links
            .backlinks
            .into_iter()
            .map(|link| VaultQualifiedLink { vault_id, link })
            .collect(),
    }
}

fn tree_for(vault_id: VaultId, snapshot: &VaultSnapshotRead) -> VaultExplorerFolder {
    let mut root = FolderBuilder::default();
    for note in &snapshot.notes {
        let segments = note.relative_path.split('/').collect::<Vec<_>>();
        root.insert(
            &segments[..segments.len().saturating_sub(1)],
            VaultExplorerNote {
                vault_id,
                title: note.title.clone(),
                slug: note.slug.clone(),
            },
        );
    }
    root.build("Vault")
}

fn graph_nodes(vault_id: VaultId, snapshot: &VaultSnapshotRead) -> Vec<VaultGraphNode> {
    let mut backlinks = BTreeMap::<&str, usize>::new();
    for link in &snapshot.links {
        *backlinks.entry(&link.target_slug).or_default() += 1;
    }
    let mut nodes = snapshot
        .notes
        .iter()
        .map(|note| VaultGraphNode {
            vault_id,
            slug: note.slug.clone(),
            title: note.title.clone(),
            primary_tag: snapshot
                .tags_by_note
                .get(&note.slug)
                .and_then(|tags| tags.first().cloned()),
            backlink_count: backlinks
                .get(note.slug.as_str())
                .copied()
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        left.title
            .cmp(&right.title)
            .then(left.slug.cmp(&right.slug))
    });
    nodes
}

#[derive(Default)]
struct FolderBuilder {
    folders: BTreeMap<String, FolderBuilder>,
    notes: Vec<VaultExplorerNote>,
}

impl FolderBuilder {
    fn insert(&mut self, segments: &[&str], note: VaultExplorerNote) {
        let Some((head, tail)) = segments.split_first() else {
            self.notes.push(note);
            return;
        };
        self.folders
            .entry((*head).to_string())
            .or_default()
            .insert(tail, note);
    }

    fn build(mut self, name: &str) -> VaultExplorerFolder {
        self.notes
            .sort_by(|left, right| left.title.cmp(&right.title));
        VaultExplorerFolder {
            name: name.to_string(),
            folders: self
                .folders
                .into_iter()
                .map(|(name, folder)| folder.build(&name))
                .collect(),
            notes: self.notes,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::TempDir;

    use super::{VaultParticipantState, VaultReadCore, VaultScope};
    use crate::cache::SqliteCache;
    use crate::embed::StubEmbedder;
    use crate::vault::VaultIndex;
    use crate::vault_registry::{NewVaultDefinition, VaultId, VaultRegistryStore, VaultSource};
    use crate::vault_runtime::VaultCollectionRuntime;

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
        let mut snapshot = None;
        let mut vault_ids = Vec::new();
        let mut vault_paths = Vec::new();
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
            let vault_id = next
                .definitions()
                .find(|definition| definition.name() == *name)
                .expect("new definition")
                .vault_id();
            vault_ids.push(vault_id);
            vault_paths.push(path);
            snapshot = Some(next);
        }
        let snapshot =
            snapshot.unwrap_or_else(|| match store.load().expect("load empty registry") {
                crate::vault_registry::VaultRegistryState::Ready(snapshot) => snapshot,
                crate::vault_registry::VaultRegistryState::Recovery(_) => {
                    panic!("unexpected recovery")
                }
            });
        runtime.reconcile(&store, &snapshot);
        let embedder = StubEmbedder::new(384);
        for (vault_id, path) in vault_ids.iter().zip(&vault_paths) {
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
            vault_paths,
        }
    }

    fn write_files(root: &Path, files: &[(&str, &str)]) {
        for (path, contents) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
            std::fs::write(path, contents).expect("write note");
        }
    }

    #[test]
    fn all_scope_keeps_equal_slugs_grouped_by_vault() {
        let workspace = workspace(&[
            (
                "First",
                &[
                    ("Home.md", "# Home\n\nfirst\n\n[[Shared]]"),
                    ("Shared.md", "# Shared"),
                ],
            ),
            (
                "Second",
                &[
                    ("Home.md", "# Home\n\nsecond\n\n[[Shared]]"),
                    ("Shared.md", "# Shared"),
                ],
            ),
        ]);
        let reads = VaultReadCore::new(&workspace.cache, &workspace.vaults);

        let trees = reads.trees(VaultScope::All).expect("trees");
        assert_eq!(trees.data.len(), 2);
        assert!(trees.data.iter().all(|tree| {
            tree.tree
                .notes
                .iter()
                .all(|note| note.vault_id == tree.vault_id)
        }));

        let statistics = reads.statistics(VaultScope::All).expect("statistics");
        assert_eq!(statistics.data.len(), 2);
        assert!(statistics.data.iter().all(|stats| stats.note_count == 2));

        let graphs = reads.graphs(VaultScope::All).expect("graphs");
        assert_eq!(graphs.data.len(), 2);
        assert!(graphs.data.iter().all(|graph| {
            graph
                .edges
                .iter()
                .all(|edge| edge.vault_id == graph.vault_id)
        }));
        assert_eq!(
            graphs
                .data
                .iter()
                .map(|graph| graph.edges.len())
                .sum::<usize>(),
            2
        );

        let recent = reads
            .recently_modified(VaultScope::All, 10)
            .expect("recent");
        assert_eq!(recent.data.len(), 4);
        assert!(
            recent
                .data
                .iter()
                .any(|note| note.vault_id == workspace.vault_ids[0] && note.slug == "home")
        );
        assert!(
            recent
                .data
                .iter()
                .any(|note| note.vault_id == workspace.vault_ids[1] && note.slug == "home")
        );
    }

    #[test]
    fn zero_enabled_vaults_is_a_complete_empty_projection() {
        let workspace = workspace(&[]);
        let reads = VaultReadCore::new(&workspace.cache, &workspace.vaults);

        let projection = reads.trees(VaultScope::All).expect("zero Vault projection");
        assert!(projection.data.is_empty());
        assert!(projection.participants.is_empty());
        assert!(!projection.partial);
    }

    #[test]
    fn one_vault_stale_snapshot_is_explicit_but_missing_snapshot_is_unavailable() {
        let workspace = workspace(&[
            ("First", &[("Home.md", "# Home\n\nfirst")]),
            ("Second", &[("Home.md", "# Home\n\nsecond")]),
        ]);
        let first = workspace.vault_ids[0];
        let second = workspace.vault_ids[1];
        let pending = VaultIndex::build(&workspace.vault_paths[0]).expect("pending index");
        std::fs::remove_file(workspace.vault_paths[0].join("Home.md"))
            .expect("break pending index");
        let failed =
            workspace
                .cache
                .replace_vault_snapshot(first, &pending, &StubEmbedder::new(384));
        assert!(
            failed.is_err(),
            "failed reindex must keep the prior snapshot stale"
        );
        workspace
            .cache
            .disconnect_vault_snapshot(second)
            .expect("remove second snapshot");

        let reads = VaultReadCore::new(&workspace.cache, &workspace.vaults);
        let stale = reads
            .trees(VaultScope::One(first))
            .expect("stale projection");
        assert!(stale.partial);
        assert_eq!(stale.participants[0].state, VaultParticipantState::Stale);
        assert_eq!(stale.data[0].tree.notes[0].slug, "home");

        let aggregate = reads.trees(VaultScope::All).expect("partial aggregate");
        assert!(aggregate.partial);
        assert_eq!(aggregate.data.len(), 1);
        assert_eq!(aggregate.participants.len(), 2);
        assert!(
            aggregate
                .participants
                .iter()
                .any(|participant| participant.vault_id == second
                    && participant.state == VaultParticipantState::Unavailable)
        );

        let unavailable = reads
            .trees(VaultScope::One(second))
            .expect_err("missing snapshot");
        assert_eq!(unavailable.code, "vault_unavailable");
        assert_eq!(unavailable.vault_id, Some(second));
    }

    #[test]
    fn exact_note_reads_stay_within_the_requested_vault() {
        let workspace = workspace(&[
            (
                "First",
                &[
                    ("Home.md", "# Home\n\nfirst\n\n[[Shared]]"),
                    ("Shared.md", "# Shared"),
                ],
            ),
            (
                "Second",
                &[
                    ("Home.md", "# Home\n\nsecond\n\n[[Shared]]"),
                    ("Shared.md", "# Shared"),
                ],
            ),
        ]);
        let reads = VaultReadCore::new(&workspace.cache, &workspace.vaults);
        let first = workspace.vault_ids[0];
        let second = workspace.vault_ids[1];

        let first_note = reads
            .exact_note(first, "home")
            .expect("first read")
            .expect("home");
        let second_note = reads
            .exact_note(second, "home")
            .expect("second read")
            .expect("home");
        assert_eq!(first_note.vault_id, first);
        assert_eq!(second_note.vault_id, second);
        assert!(first_note.note.content.contains("first"));
        assert!(second_note.note.content.contains("second"));

        let links = reads
            .exact_note_links(first, "home")
            .expect("links")
            .expect("home links");
        assert!(links.outgoing.iter().all(|link| link.vault_id == first));
        let resolved = reads
            .resolve_wikilink(second, "Shared")
            .expect("resolve")
            .expect("shared");
        assert_eq!(resolved.vault_id, second);
    }
}
