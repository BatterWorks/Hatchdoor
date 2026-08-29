//! Shared-core, Vault-qualified reads over authoritative Markdown and the
//! disposable shared SQLite snapshot cache.

use schemars::JsonSchema;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize, Serializer};
use std::str::FromStr;

use crate::cache::{SqliteCache, vault_snapshots::VaultSnapshotRead};
use crate::search::LayerSelection;
use crate::vault::{Note, NoteLink, NoteLinks};
use crate::vault_error::VaultOperationError;
use crate::vault_registry::VaultId;
use crate::vault_runtime::{VaultCapabilities, VaultCollectionRuntime};

mod assets;

pub(crate) use assets::{AssetPathError, AssetReadError, ResolvedAsset, asset_download_path};

/// An explicit collection read target. There is deliberately no selected,
/// default, or sole-Vault variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultScope {
    One(VaultId),
    All,
}

impl VaultScope {
    /// Parse the `scope` a caller supplies — the literal `all`, or a canonical
    /// Vault ID. One implementation for both adapters: HTTP takes it from the
    /// `{scope}` path segment and MCP from a tool argument, and a scope one
    /// surface accepts must not be a scope the other refuses. Anything else is
    /// the structured `invalid_scope` failure.
    pub fn parse(raw: &str) -> Result<Self, VaultReadError> {
        if raw == "all" {
            return Ok(VaultScope::All);
        }
        VaultId::from_str(raw)
            .map(VaultScope::One)
            .map_err(|_| VaultReadError {
                code: "invalid_scope".to_string(),
                message: "scope must be the literal 'all' or a canonical Vault ID".to_string(),
                vault_id: None,
                retryable: false,
            })
    }
}

/// The default and bounds every collection read applies to `limit`, shared so
/// the two adapters cannot clamp differently. They mirror the legacy
/// `/api/recently-modified` and `/api/search` behaviour the Vault-scoped routes
/// replaced, and match the bounds the MCP tool schemas advertise.
pub(crate) fn clamp_recent_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(5).clamp(1, 25)
}

pub(crate) fn clamp_search_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(10).clamp(1, 50)
}

pub(crate) fn clamp_search_per_note_cap(per_note_cap: Option<usize>) -> usize {
    per_note_cap.unwrap_or(2).clamp(1, 10)
}

/// The `note_not_found` failure both adapters report when an exact read
/// resolves its slug to nothing on the caller's browse surface. Shared so a
/// Note withheld by the demo surface (#109) is indistinguishable from an absent
/// one on either surface, down to the message.
pub(crate) fn note_not_found(vault_id: VaultId, slug: &str) -> VaultOperationError {
    VaultOperationError::new(
        "note_not_found",
        format!("Note not found: {slug}"),
        Some(vault_id),
        false,
    )
}

/// Serializes as the flat scalar `docs/migrations/vault-scoped-clients.md`'s
/// envelope documents (the Vault ID's canonical text, or the literal
/// `"all"`), mirroring exactly what a caller passes as the `scope` path
/// segment — not serde's derived externally-tagged shape (`{"one": "<uuid>"}`
/// for the data-carrying variant), which no wire consumer expects.
impl Serialize for VaultScope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            VaultScope::One(vault_id) => vault_id.serialize(serializer),
            VaultScope::All => serializer.serialize_str("all"),
        }
    }
}

/// Deserializes the same flat scalar `Serialize` emits (the canonical text of
/// a Vault ID, or the literal `"all"`), so a typed result can round-trip.
impl<'de> Deserialize<'de> for VaultScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value == "all" {
            return Ok(VaultScope::All);
        }
        VaultId::from_str(&value)
            .map(VaultScope::One)
            .map_err(|_| serde::de::Error::custom("invalid scope"))
    }
}

impl JsonSchema for VaultScope {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "VaultScope".into()
    }

    fn json_schema(_generator: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "A canonical Vault ID or the literal all.",
            "examples": ["all"]
        })
    }

    fn inline_schema() -> bool {
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultReadError {
    pub code: String,
    pub message: String,
    pub vault_id: Option<VaultId>,
    pub retryable: bool,
}

impl VaultReadError {
    /// The stable, caller-facing `code` for this failure.
    ///
    /// The list is exactly what this core can produce, so a code neither
    /// surface knows never leaks as a novel one: the core's own internal
    /// spelling `vault_runtime_not_active`, and anything unforeseen, collapse
    /// to `vault_unavailable`. Adapters go through this rather than each
    /// keeping their own translation table. Widen it only alongside the status
    /// and meaning each adapter should give the new code.
    pub fn public_code(&self) -> &str {
        match self.code.as_str() {
            // Vault gating, collection reads, and search.
            "vault_not_found"
            | "vault_disabled"
            | "vault_scan_config_invalid"
            | "vault_read_unavailable"
            | "invalid_scope"
            | "invalid_search_query"
            | "invalid_layer_selection"
            | "search_unavailable"
            // Exact reads of one Note's contents.
            | "note_not_found"
            | "note_unreadable"
            | "invalid_frontmatter"
            // `note_attachments` reads the Note through the write module's
            // attachment lister, whose only failure on a read is I/O.
            | "write_failed" => self.code.as_str(),
            _ => "vault_unavailable",
        }
    }

    /// This failure in the `{code, message, vault_id?, retryable}` shape every
    /// surface reports, with its public code applied.
    pub fn into_operation_error(self) -> VaultOperationError {
        let code = self.public_code().to_string();
        VaultOperationError {
            code,
            message: self.message,
            vault_id: self.vault_id,
            retryable: self.retryable,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultParticipantState {
    Fresh,
    Stale,
    /// The Vault's rows are current, but this generation carries no vectors,
    /// so it contributed nothing to a semantic search. Only semantic search
    /// reports it: browsing, keyword and tag search all read the same
    /// structural rows and report `Fresh`.
    ///
    /// Distinct from `Unavailable`, which means there is nothing to read at
    /// all. Collapsing the two would tell a caller its Notes are missing when
    /// they are merely not yet embedded.
    NotSearchable,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultParticipant {
    pub vault_id: VaultId,
    pub vault_name: String,
    pub state: VaultParticipantState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<VaultReadError>,
}

/// The common one-or-all read envelope future HTTP and MCP adapters share.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultReadProjection<T> {
    pub scope: VaultScope,
    pub collection_revision: u64,
    pub partial: bool,
    pub participants: Vec<VaultParticipant>,
    pub data: T,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultQualifiedNote {
    pub vault_id: VaultId,
    pub note: Note,
}

/// The rich, exact single-Vault statistics report — every field the legacy
/// single-Vault `/api/stats` response computed. `stats` reuses
/// `api_types::VaultStatsResponse` directly rather than duplicating its wire
/// shape; `vault_id` sits alongside it, matching `VaultQualifiedNote`'s
/// nesting rather than a flat merge.
#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct VaultQualifiedStats {
    pub vault_id: VaultId,
    pub stats: crate::api_types::VaultStatsResponse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultQualifiedLink {
    pub vault_id: VaultId,
    pub link: NoteLink,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultQualifiedLinks {
    pub vault_id: VaultId,
    pub outgoing: Vec<VaultQualifiedLink>,
    pub backlinks: Vec<VaultQualifiedLink>,
}

/// One batch's resolutions, positionally matching the note targets and the
/// asset targets that were asked for. Assets carry a Vault-relative path
/// because they have no slug to name them by.
pub type ResolvedVaultTargets = (Vec<Option<ResolvedVaultNote>>, Vec<Option<String>>);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct ResolvedVaultNote {
    pub vault_id: VaultId,
    pub slug: String,
    pub relative_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultTree {
    pub vault_id: VaultId,
    pub vault_name: String,
    pub tree: VaultExplorerFolder,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultExplorerFolder {
    pub name: String,
    pub folders: Vec<VaultExplorerFolder>,
    pub notes: Vec<VaultExplorerNote>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultExplorerNote {
    pub vault_id: VaultId,
    pub title: String,
    pub slug: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultStatistics {
    pub vault_id: VaultId,
    pub vault_name: String,
    pub note_count: usize,
    pub tag_count: usize,
    pub link_count: usize,
    pub vault_size_bytes: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultGraph {
    pub vault_id: VaultId,
    pub vault_name: String,
    pub nodes: Vec<VaultGraphNode>,
    pub edges: Vec<VaultGraphEdge>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultGraphNode {
    pub vault_id: VaultId,
    pub slug: String,
    pub title: String,
    pub primary_tag: Option<String>,
    pub backlink_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultGraphEdge {
    pub vault_id: VaultId,
    pub source_slug: String,
    pub target_slug: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct VaultRecentNote {
    pub vault_id: VaultId,
    pub title: String,
    pub slug: String,
    pub relative_path: String,
    pub mtime_ns: i64,
}

/// Which layer surface a [`VaultReadCore`] is allowed to see.
///
/// An ordinary instance browses `Everything`: layers demote a Note from the
/// default *search* surface, but the operator still reaches it by slug, in the
/// explorer, and on the graph. A public read-only demo (#109) has no operator
/// and no layer toggle, so demoted Notes are withheld from every read instead:
/// they cannot be fetched, searched, resolved, downloaded, or inferred from a
/// tree, graph, recent list, or statistic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseSurface {
    /// Every Note, demoted or not. The ordinary authenticated instance.
    Everything,
    /// Only Notes on the default surface (`layer IS NULL`).
    DefaultOnly,
}

impl BrowseSurface {
    /// The surface a request to this instance may browse. Demo mode is the
    /// only thing that narrows it: #109 makes a demo a browsing product with
    /// no operator and no layer toggle.
    pub fn for_demo_mode(demo_mode: bool) -> Self {
        if demo_mode {
            Self::DefaultOnly
        } else {
            Self::Everything
        }
    }

    /// Whether a Note carrying `layer` is withheld from this surface.
    fn hides(self, layer: Option<&str>) -> bool {
        self == Self::DefaultOnly && layer.is_some()
    }

    /// The [`LayerSelection`] a collection read applies, from the caller's raw
    /// comma-separated tokens (`"all"`, `"default"`, or exact layer names).
    /// One implementation for both adapters: HTTP reads the tokens from a
    /// `layers=` query and MCP joins its `layers` array, and a selector one
    /// surface honours must not mean something else on the other.
    ///
    /// Deliberately does not consult any one Vault's known-layer catalog while
    /// parsing, unlike [`LayerSelection::parse`] (built for the single-Vault
    /// surface, where an unrecognized token degrades to the default surface
    /// with a warning): issue #62 applies one layer selector *independently* to
    /// every participant, so a name valid in one Vault and absent from another
    /// is not a parse-time concern — `VaultSearchCore::search`'s own
    /// `invalid_layer_selection` check already covers the only real error case,
    /// a named layer absent from every usable participant.
    ///
    /// `DefaultOnly` ignores the tokens entirely (#109): a demo has no operator
    /// and no layer toggle, so the selector is not an escape hatch out of the
    /// default surface. Clamping rather than rejecting keeps a saved link with
    /// `?layers=` working and returning what an unadorned read would, so the
    /// demoted Notes' existence cannot be inferred from a differing error.
    pub fn layer_selection(self, raw: Option<&str>) -> LayerSelection {
        if self == Self::DefaultOnly {
            return LayerSelection::default_surface();
        }
        let Some(raw) = raw else {
            return LayerSelection::default_surface();
        };
        let tokens: Vec<&str> = raw
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .collect();
        if tokens.is_empty() {
            return LayerSelection::default_surface();
        }
        if tokens.iter().any(|token| token.eq_ignore_ascii_case("all")) {
            return LayerSelection::All;
        }
        let mut include_default = false;
        let mut layers = BTreeSet::new();
        for token in tokens {
            if token.eq_ignore_ascii_case("default") {
                include_default = true;
            } else {
                layers.insert(token.to_string());
            }
        }
        if !include_default && layers.is_empty() {
            include_default = true;
        }
        LayerSelection::Set {
            include_default,
            layers,
        }
    }

    /// Drop every demoted row from one published snapshot before anything
    /// projects it, so a restricted surface (#109's demo mode) cannot reveal a
    /// demoted Note through a tree, graph, recent list, statistic, or a search
    /// result's outbound links. Links are dropped when *either* endpoint is
    /// withheld: a surviving edge would name the hidden Note's slug.
    pub(crate) fn restrict(self, read: VaultSnapshotRead) -> VaultSnapshotRead {
        if self == Self::Everything {
            return read;
        }
        let VaultSnapshotRead {
            notes,
            links,
            mut tags_by_note,
            chunks,
            ..
        } = read;
        let notes: Vec<_> = notes
            .into_iter()
            .filter(|note| !self.hides(note.layer.as_deref()))
            .collect();
        let visible: BTreeSet<&str> = notes.iter().map(|note| note.slug.as_str()).collect();
        let links = links
            .into_iter()
            .filter(|link| {
                visible.contains(link.source_slug.as_str())
                    && visible.contains(link.target_slug.as_str())
            })
            .collect();
        let chunks = chunks
            .into_iter()
            .filter(|chunk| {
                !self.hides(chunk.layer.as_deref()) && visible.contains(chunk.note_slug.as_str())
            })
            .collect();
        tags_by_note.retain(|slug, _| visible.contains(slug.as_str()));
        // A restricted surface can select no layer, so it publishes no
        // catalogue: an empty catalogue is also what a Vault with no markers
        // reports, keeping the two indistinguishable.
        VaultSnapshotRead {
            notes,
            links,
            tags_by_note,
            chunks,
            layer_catalog: Vec::new(),
        }
    }
}

/// One Vault's resolution of a wikilink target: the Note it names, or nothing.
///
/// A read projection rather than an HTTP response body — it began life in
/// `handlers/vault_content.rs`, which MCP reached by decoding that route's
/// response, and now sits beside the exact-read projections both adapters
/// serialize (#188).
#[derive(Debug, Serialize, JsonSchema, Deserialize)]
pub struct VaultResolveResponse {
    pub vault_id: VaultId,
    pub slug: Option<String>,
}

/// One Note's frontmatter, without its Markdown body.
#[derive(Debug, Clone)]
pub struct VaultNoteFrontmatter {
    pub slug: String,
    pub relative_path: String,
    pub has_frontmatter: bool,
    pub metadata: crate::cache::parse::FrontmatterMetadata,
}

/// Why one read run off the async runtime did not produce a value.
pub enum OffloadedReadError {
    /// The read's own structured failure.
    Read(VaultReadError),
    /// The blocking task never completed — a panic or a cancelled runtime.
    /// Distinct from `Read` because it says nothing about the Vault: it is an
    /// instance-side fault, and each adapter reports it the way it reports its
    /// own internal errors.
    Failed(String),
}

/// An owned handle to the read core, for an adapter that must run a read off
/// the async runtime.
///
/// ADR-19 puts that offload behind the core rather than in each adapter:
/// [`VaultReadCore`] borrows the cache and the runtime, so every call site
/// would otherwise repeat the same clone-and-`spawn_blocking` prologue — and
/// they had already drifted, with the MCP tools running index builds and
/// filesystem reads straight on a tokio worker while HTTP offloaded them.
#[derive(Clone)]
pub struct VaultReads {
    cache: Arc<SqliteCache>,
    vaults: VaultCollectionRuntime,
    surface: BrowseSurface,
}

impl VaultReads {
    /// The handle for one request, on the browse surface this instance serves.
    pub fn new(state: &crate::app_state::AppState) -> Self {
        Self {
            cache: state.startup_sqlite.clone(),
            vaults: state.vaults.clone(),
            surface: BrowseSurface::for_demo_mode(state.demo_mode),
        }
    }

    pub(crate) fn surface(&self) -> BrowseSurface {
        self.surface
    }

    /// Run one read off the async runtime and hand back its outcome. The work
    /// returns the core's own failure; only this handle can produce the
    /// task-never-completed one.
    pub async fn read<T, F>(&self, work: F) -> Result<T, OffloadedReadError>
    where
        F: FnOnce(&VaultReadCore<'_>) -> Result<T, VaultReadError> + Send + 'static,
        T: Send + 'static,
    {
        let cache = self.cache.clone();
        let vaults = self.vaults.clone();
        let surface = self.surface;
        match tokio::task::spawn_blocking(move || {
            work(&VaultReadCore::new(&cache, &vaults).on_surface(surface))
        })
        .await
        {
            Ok(outcome) => outcome.map_err(OffloadedReadError::Read),
            Err(join_error) => Err(OffloadedReadError::Failed(format!(
                "background task panicked: {join_error}"
            ))),
        }
    }
}

/// The shared-core facade. Exact reads use the targeted Vault's current
/// Markdown directory; collection projections use only already-published
/// cache snapshots and report their freshness honestly.
pub struct VaultReadCore<'a> {
    cache: &'a SqliteCache,
    vaults: &'a VaultCollectionRuntime,
    surface: BrowseSurface,
}

impl<'a> VaultReadCore<'a> {
    pub fn new(cache: &'a SqliteCache, vaults: &'a VaultCollectionRuntime) -> Self {
        Self {
            cache,
            vaults,
            surface: BrowseSurface::Everything,
        }
    }

    /// Restrict every read to the default surface. Demo mode (#109) is the only
    /// production caller: a visitor has no way to reveal a demoted Note, so the
    /// server must not serve one through any surface.
    pub fn on_surface(mut self, surface: BrowseSurface) -> Self {
        self.surface = surface;
        self
    }

    pub fn exact_note(
        &self,
        vault_id: VaultId,
        slug: &str,
    ) -> Result<Option<VaultQualifiedNote>, VaultReadError> {
        let index = self.authoritative_index(vault_id)?;
        index
            .read_note_by_slug(slug)
            .map(|note| {
                note.filter(|note| !self.surface.hides(note.layer.as_deref()))
                    .map(|note| VaultQualifiedNote { vault_id, note })
            })
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
        if self.hidden_slug(&index, slug) {
            return Ok(None);
        }
        Ok(index.note_links(slug).map(|mut links| {
            if self.surface == BrowseSurface::DefaultOnly {
                links
                    .outgoing
                    .retain(|link| !self.surface.hides(link.layer.as_deref()));
                links
                    .backlinks
                    .retain(|link| !self.surface.hides(link.layer.as_deref()));
            }
            qualify_links(vault_id, links)
        }))
    }

    pub fn resolve_wikilink(
        &self,
        vault_id: VaultId,
        raw_target: &str,
    ) -> Result<Option<ResolvedVaultNote>, VaultReadError> {
        let index = self.authoritative_index(vault_id)?;
        Ok(index
            .resolve_wikilink(raw_target)
            .filter(|note| !self.surface.hides(note.layer.as_deref()))
            .map(|note| ResolvedVaultNote {
                vault_id,
                slug: note.slug.clone(),
                relative_path: note.relative_path.clone(),
            }))
    }

    /// Resolve every target against one authoritative index build, for
    /// batch-resolve adapters. `resolve_wikilink` builds a fresh index per
    /// call, which is correct for one target but would otherwise cost a full
    /// Vault scan per batch entry.
    pub fn resolve_wikilinks(
        &self,
        vault_id: VaultId,
        raw_targets: &[String],
    ) -> Result<Vec<Option<ResolvedVaultNote>>, VaultReadError> {
        Ok(self.resolve_batch(vault_id, raw_targets, &[], "")?.0)
    }

    /// Resolve a note's wikilink targets — notes and assets alike — against one
    /// authoritative-index build.
    ///
    /// Assets resolve separately from notes because they are addressed
    /// differently: a note has a slug, an asset only ever has a path, and an
    /// Obsidian-authored embed names it by filename alone (#158). `note_dir` is
    /// the Vault-relative directory of the note the targets were written in,
    /// which decides both the relative reading and which of several namesakes
    /// is nearest; `""` is the Vault root.
    pub fn resolve_batch(
        &self,
        vault_id: VaultId,
        note_targets: &[String],
        asset_targets: &[String],
        note_dir: &str,
    ) -> Result<ResolvedVaultTargets, VaultReadError> {
        let index = self.authoritative_index(vault_id)?;
        let notes = note_targets
            .iter()
            .map(|raw_target| {
                index
                    .resolve_wikilink(raw_target)
                    .filter(|note| !self.surface.hides(note.layer.as_deref()))
                    .map(|note| ResolvedVaultNote {
                        vault_id,
                        slug: note.slug.clone(),
                        relative_path: note.relative_path.clone(),
                    })
            })
            .collect();
        // Assets carry no layer, so the browse surface has nothing to hide
        // here: an embed only resolves for a caller already reading the note
        // that contains it.
        let assets = asset_targets
            .iter()
            .map(|raw_target| {
                index
                    .resolve_asset(raw_target, note_dir)
                    .map(str::to_string)
            })
            .collect();
        Ok((notes, assets))
    }

    /// Whether this surface withholds `slug` entirely, so a caller answers the
    /// same not-found it would give a Note that does not exist. Keeping the
    /// two indistinguishable is the point: a demo visitor must not be able to
    /// infer a demoted Note's existence from a different error.
    fn hidden_slug(&self, index: &crate::vault::VaultIndex, slug: &str) -> bool {
        index
            .find_by_slug(slug)
            .is_some_and(|entry| self.surface.hides(entry.layer.as_deref()))
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

    /// The rich per-Vault statistics report, scoped to exactly one Vault —
    /// never `all`. This is an exact read, like `exact_note`, not a `{scope}`
    /// collection projection: it returns the report directly rather than a
    /// participants/partial envelope. It reuses `collection`'s
    /// `VaultScope::One` gating for the identical not-found/disabled/
    /// unavailable behavior the lean `statistics` (`{scope}/stats`) endpoint
    /// already has, computed from the same published snapshot `statistics`
    /// reads rather than the legacy single-Vault-shaped SQL cache tables the
    /// retired scope-less statistics query read.
    pub fn statistics_detail(
        &self,
        vault_id: VaultId,
    ) -> Result<VaultQualifiedStats, VaultReadError> {
        let projection = self.collection(
            VaultScope::One(vault_id),
            |_vault_id, _vault_name, snapshot| detailed_stats_for(snapshot),
        )?;
        let stats = projection
            .data
            .into_iter()
            .next()
            .expect("VaultScope::One yields exactly one participant on success");
        Ok(VaultQualifiedStats { vault_id, stats })
    }

    /// The requested Vault's resolved local Markdown directory, gated by the
    /// same not-found/disabled/unavailable checks as `authoritative_index`,
    /// without paying the cost of parsing every note. For adapters (contained
    /// asset/attachment/download serving) that only need the directory.
    ///
    /// Explicitly confirms the directory currently exists on disk (a managed
    /// Git Vault can be enabled and accepting operations before its checkout
    /// has materialized) so a transient-unavailable Vault reports the same
    /// retryable `vault_read_unavailable` code an exact-note read would,
    /// rather than callers discovering a raw filesystem error later and
    /// reporting an unrelated, non-retryable status.
    pub fn vault_directory(&self, vault_id: VaultId) -> Result<std::path::PathBuf, VaultReadError> {
        let control = self.control_block(vault_id)?;
        control
            .ensure_accepting_operations()
            .map_err(|error| runtime_error(vault_id, error))?;
        let path = control.vault_path();
        if std::fs::metadata(path).is_err() {
            return Err(unavailable(
                vault_id,
                "vault_read_unavailable",
                format!(
                    "Vault {vault_id}'s local Markdown directory is not currently available at \
                     '{}'",
                    path.display()
                ),
                true,
            ));
        }
        Ok(path.to_path_buf())
    }

    /// What this Vault's own source mode and lifecycle phase currently allow,
    /// under the same not-found/disabled/no-runtime gate every other read
    /// applies. For an adapter reporting a Vault's posture rather than
    /// exercising it (the MCP `get_attachment_import_config` tool).
    pub fn vault_capabilities(
        &self,
        vault_id: VaultId,
    ) -> Result<VaultCapabilities, VaultReadError> {
        Ok(self.control_block(vault_id)?.snapshot().capabilities)
    }

    /// One Note's frontmatter — tags, aliases, and properties — without its
    /// Markdown body, on this core's browse surface. `Ok(None)` is a Note this
    /// caller may not see, whether it is absent or withheld.
    ///
    /// Reading the note's bytes is the same read `exact_note` performs; the
    /// projection just never returns them, and the canonical cache-layer
    /// parser decides what counts as frontmatter.
    pub fn exact_note_frontmatter(
        &self,
        vault_id: VaultId,
        slug: &str,
    ) -> Result<Option<VaultNoteFrontmatter>, VaultReadError> {
        let index = self.authoritative_index(vault_id)?;
        let Some(entry) = self.visible_entry(&index, slug) else {
            return Ok(None);
        };
        let content = std::fs::read_to_string(&entry.path).map_err(|error| {
            unavailable(
                vault_id,
                "note_unreadable",
                format!("failed to read note '{}': {error}", entry.relative_path),
                false,
            )
        })?;
        let has_frontmatter = crate::cache::parse::frontmatter_span(&content).is_some();
        let metadata = crate::cache::parse::parse_frontmatter_metadata(&content)
            .map_err(|message| unavailable(vault_id, "invalid_frontmatter", message, false))?;
        Ok(Some(VaultNoteFrontmatter {
            slug: entry.slug.clone(),
            relative_path: entry.relative_path.clone(),
            has_frontmatter,
            metadata,
        }))
    }

    /// The existing attachments one Note references, on this core's browse
    /// surface. `Ok(None)` is a Note this caller may not see.
    pub fn note_attachments(
        &self,
        vault_id: VaultId,
        slug: &str,
    ) -> Result<Option<Vec<crate::vault::AttachmentInfo>>, VaultReadError> {
        let (control, index) = self.control_and_index(vault_id)?;
        let Some(entry) = self.visible_entry(&index, slug) else {
            return Ok(None);
        };
        crate::vault::list_note_attachments(control.vault_path(), &index.layers, &entry)
            .map(Some)
            .map_err(|error| {
                let error = crate::vault_mutation::write_operation_error(vault_id, error);
                VaultReadError {
                    code: error.code,
                    message: error.message,
                    vault_id: error.vault_id,
                    retryable: error.retryable,
                }
            })
    }

    /// The Note entry `slug` names, or nothing when this surface withholds it.
    /// Withheld and absent are deliberately the same answer: a demo visitor
    /// must not be able to infer a demoted Note's existence from a different
    /// outcome.
    fn visible_entry(
        &self,
        index: &crate::vault::VaultIndex,
        slug: &str,
    ) -> Option<crate::vault::NoteEntry> {
        index
            .find_by_slug(slug.trim())
            .filter(|entry| !self.surface.hides(entry.layer.as_deref()))
            .cloned()
    }

    /// One contained attachment or embedded asset, resolved against the
    /// requested Vault's gated Markdown directory and described without being
    /// read.
    ///
    /// This is the single home for the contained-resource policy both surfaces
    /// answer on (#188): the Vault gate, path containment against the canonical
    /// root, the servable-extension allow-list, the content-type table, and the
    /// browse surface. The outer error is the Vault's own; the inner one is the
    /// path's, which each adapter maps to its wire shape.
    pub(crate) fn contained_asset(
        &self,
        vault_id: VaultId,
        relative_path: &str,
    ) -> Result<Result<ResolvedAsset, AssetPathError>, VaultReadError> {
        let vault_root = self.vault_directory(vault_id)?;
        let asset = match assets::describe_asset(&vault_root, relative_path) {
            Ok(asset) => asset,
            Err(error) => return Ok(Err(error)),
        };
        if !self.asset_on_surface(vault_id, &asset.relative_path)? {
            return Ok(Err(AssetPathError::NotFound));
        }
        Ok(Ok(asset))
    }

    /// Whether a contained asset path belongs to this core's selected browse
    /// surface. An ordinary instance retains the legacy contained-asset
    /// behavior. A demo accepts only an asset present in the authoritative
    /// index's asset catalog, which has already applied the complete
    /// exclusion/noise policy, and then applies the layer map to that path.
    /// Assets do not carry a layer of their own, so this is the same
    /// path-to-surface decision the index uses for Notes.
    pub fn asset_on_surface(
        &self,
        vault_id: VaultId,
        relative_path: &str,
    ) -> Result<bool, VaultReadError> {
        if self.surface == BrowseSurface::Everything {
            return Ok(true);
        }
        let index = self.authoritative_index(vault_id)?;
        Ok(index.asset_paths.contains(relative_path)
            && !self.surface.hides(index.layers.layer_for(relative_path)))
    }

    /// The exact Note together with the local Markdown directory it was read
    /// from, both drawn from one Vault control-block fetch. A caller that
    /// needs both (e.g. to zip a note with its referenced assets) must not
    /// call `exact_note` and `vault_directory` separately: a concurrent Vault
    /// edit reconciles a *replacement* control block rather than mutating the
    /// current one in place, so two independent lookups could observe the
    /// note from one Vault path and resolve assets against another.
    pub fn exact_note_for_download(
        &self,
        vault_id: VaultId,
        slug: &str,
    ) -> Result<Option<(VaultQualifiedNote, std::path::PathBuf)>, VaultReadError> {
        let (control, index) = self.control_and_index(vault_id)?;
        index
            .read_note_by_slug(slug)
            .map(|note| {
                note.filter(|note| !self.surface.hides(note.layer.as_deref()))
                    .map(|note| {
                        (
                            VaultQualifiedNote { vault_id, note },
                            control.vault_path().to_path_buf(),
                        )
                    })
            })
            .map_err(|error| {
                unavailable(vault_id, "vault_read_unavailable", error.to_string(), true)
            })
    }

    /// The gated Vault control block: not-found, disabled, and no-runtime all
    /// resolve here, and callers that go on to build an index or read the
    /// filesystem must also apply an accepting-operations/existence check of
    /// their own kind, since only `control_and_index` bundles the exact-read
    /// gate and `vault_directory` bundles the directory-existence gate.
    ///
    /// Widened to `pub(crate)` for `handlers/vault_write.rs` (#101), the first
    /// mutation adapter: mutations need the identical not-found/disabled/
    /// no-runtime gate exact reads already apply, rather than a duplicated
    /// copy of this match.
    pub(crate) fn control_block(
        &self,
        vault_id: VaultId,
    ) -> Result<crate::vault_runtime::VaultControlBlock, VaultReadError> {
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
        self.vaults.runtime(vault_id).ok_or_else(|| {
            unavailable(
                vault_id,
                "vault_unavailable",
                "Vault has no active runtime".to_string(),
                true,
            )
        })
    }

    /// The gated control block together with its freshly built authoritative
    /// index, shared by every exact-read method that needs a parsed index
    /// (`authoritative_index` discards the control block;
    /// `exact_note_for_download` keeps it for `vault_path()`).
    fn control_and_index(
        &self,
        vault_id: VaultId,
    ) -> Result<
        (
            crate::vault_runtime::VaultControlBlock,
            crate::vault::VaultIndex,
        ),
        VaultReadError,
    > {
        let control = self.control_block(vault_id)?;
        let index = control
            .authoritative_index()
            .map_err(|error| runtime_error(vault_id, error))?;
        Ok((control, index))
    }

    fn authoritative_index(
        &self,
        vault_id: VaultId,
    ) -> Result<crate::vault::VaultIndex, VaultReadError> {
        self.control_and_index(vault_id).map(|(_, index)| index)
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
                    let read = self.surface.restrict(published.read);
                    data.push(map(selected.vault_id, &selected.vault_name, &read));
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

/// Widened to `pub(crate)` for `handlers/vault_write.rs` (#101): mutations
/// surface the same `VaultControlBlock` runtime errors (e.g. from
/// `acquire_mutation`) that exact reads do, through the same `VaultReadError`
/// shape and `vault_read_error_response` mapping.
pub(crate) fn runtime_error(
    vault_id: VaultId,
    error: crate::vault_runtime::VaultRuntimeError,
) -> VaultReadError {
    VaultReadError {
        code: error.code,
        message: error.message,
        vault_id: Some(vault_id),
        retryable: error.retryable,
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

/// Computes every `VaultStatsResponse` field from one Vault's published
/// snapshot. A Rust port of the retired scope-less statistics query's SQL,
/// rewritten against `VaultSnapshotRead` because the multi-Vault architecture
/// publishes Vault-attributed data there rather than through that legacy
/// single-Vault-shaped cache schema. Unlike the legacy function, this applies no
/// `LayerSelection` filter, matching `VaultReadCore::statistics`'s existing
/// lean projection: every note in the published snapshot counts, consistent
/// with what that already-shipped collection endpoint reports for the same
/// Vault.
fn detailed_stats_for(snapshot: &VaultSnapshotRead) -> crate::api_types::VaultStatsResponse {
    use crate::api_types::{
        FolderStat, LinkedNoteRef, MonthActivity, NoteList, NoteRef, NoteWordRef, TagStat,
        VaultStatsResponse,
    };

    let note_count = snapshot.notes.len() as i64;
    let vault_size_bytes: i64 = snapshot.notes.iter().map(|note| note.size_bytes).sum();

    let mut total_word_count = 0usize;
    let mut total_image_count = 0usize;
    let mut word_counts: Vec<(&str, &str, usize)> = Vec::with_capacity(snapshot.notes.len());
    for note in &snapshot.notes {
        let word_count = word_count_for_content(&note.content);
        total_word_count += word_count;
        total_image_count += note.content.matches("![").count();
        word_counts.push((note.slug.as_str(), note.title.as_str(), word_count));
    }
    let avg_word_count = if note_count > 0 {
        total_word_count / note_count as usize
    } else {
        0
    };

    word_counts.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(b.0)));
    let longest_notes: Vec<NoteWordRef> = word_counts
        .iter()
        .take(5)
        .map(|(slug, title, word_count)| NoteWordRef {
            title: (*title).to_string(),
            slug: (*slug).to_string(),
            word_count: *word_count,
        })
        .collect();

    word_counts.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.0.cmp(b.0)));
    let shortest_notes: Vec<NoteWordRef> = word_counts
        .iter()
        .filter(|(_, _, word_count)| *word_count > 0)
        .take(5)
        .map(|(slug, title, word_count)| NoteWordRef {
            title: (*title).to_string(),
            slug: (*slug).to_string(),
            word_count: *word_count,
        })
        .collect();

    let mut tag_counts: BTreeMap<&str, i64> = BTreeMap::new();
    for note in &snapshot.notes {
        for tag in snapshot.tags_by_note.get(&note.slug).into_iter().flatten() {
            *tag_counts.entry(tag.as_str()).or_insert(0) += 1;
        }
    }
    let tag_count = tag_counts.len() as i64;
    let mut top_tags: Vec<TagStat> = tag_counts
        .into_iter()
        .map(|(tag, note_count)| TagStat {
            tag: tag.to_string(),
            note_count,
        })
        .collect();
    top_tags.sort_by(|a, b| {
        b.note_count
            .cmp(&a.note_count)
            .then_with(|| a.tag.cmp(&b.tag))
    });
    top_tags.truncate(20);

    let link_count = snapshot.links.len() as i64;

    let mut backlinks_by_target: BTreeMap<&str, i64> = BTreeMap::new();
    for link in &snapshot.links {
        *backlinks_by_target
            .entry(link.target_slug.as_str())
            .or_insert(0) += 1;
    }
    let mut most_linked: Vec<LinkedNoteRef> = snapshot
        .notes
        .iter()
        .filter_map(|note| {
            let backlink_count = *backlinks_by_target.get(note.slug.as_str())?;
            (backlink_count > 0).then(|| LinkedNoteRef {
                title: note.title.clone(),
                slug: note.slug.clone(),
                backlink_count,
            })
        })
        .collect();
    most_linked.sort_by(|a, b| {
        b.backlink_count
            .cmp(&a.backlink_count)
            .then_with(|| a.title.cmp(&b.title))
    });
    most_linked.truncate(20);

    let mut activity: BTreeMap<String, i64> = BTreeMap::new();
    for note in &snapshot.notes {
        *activity.entry(month_key(note.mtime_ns)).or_insert(0) += 1;
    }
    let mut activity_by_month: Vec<MonthActivity> = activity
        .into_iter()
        .map(|(month, modified_count)| MonthActivity {
            month,
            modified_count,
        })
        .collect();
    activity_by_month.sort_by(|a, b| b.month.cmp(&a.month));
    activity_by_month.truncate(6);

    let mut folder_counts: BTreeMap<String, i64> = BTreeMap::new();
    for note in &snapshot.notes {
        *folder_counts
            .entry(top_folder(&note.relative_path))
            .or_insert(0) += 1;
    }
    let mut notes_per_folder: Vec<FolderStat> = folder_counts
        .into_iter()
        .map(|(folder, note_count)| FolderStat { folder, note_count })
        .collect();
    notes_per_folder.sort_by(|a, b| {
        b.note_count
            .cmp(&a.note_count)
            .then_with(|| a.folder.cmp(&b.folder))
    });

    let linked_sources: BTreeSet<&str> = snapshot
        .links
        .iter()
        .map(|link| link.source_slug.as_str())
        .collect();
    let linked_targets: BTreeSet<&str> = snapshot
        .links
        .iter()
        .map(|link| link.target_slug.as_str())
        .collect();
    let mut orphan_notes: Vec<NoteRef> = snapshot
        .notes
        .iter()
        .filter(|note| {
            !linked_sources.contains(note.slug.as_str())
                && !linked_targets.contains(note.slug.as_str())
        })
        .map(|note| NoteRef {
            title: note.title.clone(),
            slug: note.slug.clone(),
        })
        .collect();
    orphan_notes.sort_by(|a, b| a.title.cmp(&b.title));

    let mut no_tag_notes: Vec<NoteRef> = snapshot
        .notes
        .iter()
        .filter(|note| {
            snapshot
                .tags_by_note
                .get(&note.slug)
                .is_none_or(|tags| tags.is_empty())
        })
        .map(|note| NoteRef {
            title: note.title.clone(),
            slug: note.slug.clone(),
        })
        .collect();
    no_tag_notes.sort_by(|a, b| a.title.cmp(&b.title));

    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    let week_threshold = now_ns.saturating_sub(7 * 86_400 * 1_000_000_000);
    let month_threshold = now_ns.saturating_sub(30 * 86_400 * 1_000_000_000);

    let mut week_notes: Vec<&crate::cache::vault_snapshots::VaultSnapshotNote> = snapshot
        .notes
        .iter()
        .filter(|note| note.mtime_ns >= week_threshold)
        .collect();
    week_notes.sort_by_key(|note| std::cmp::Reverse(note.mtime_ns));
    let week_total = week_notes.len() as i64;
    let week_notes: Vec<NoteRef> = week_notes
        .into_iter()
        .take(20)
        .map(|note| NoteRef {
            title: note.title.clone(),
            slug: note.slug.clone(),
        })
        .collect();

    let mut month_notes: Vec<&crate::cache::vault_snapshots::VaultSnapshotNote> = snapshot
        .notes
        .iter()
        .filter(|note| note.mtime_ns >= month_threshold)
        .collect();
    month_notes.sort_by_key(|note| std::cmp::Reverse(note.mtime_ns));
    let month_total = month_notes.len() as i64;
    let month_notes: Vec<NoteRef> = month_notes
        .into_iter()
        .take(20)
        .map(|note| NoteRef {
            title: note.title.clone(),
            slug: note.slug.clone(),
        })
        .collect();

    VaultStatsResponse {
        note_count,
        word_count: total_word_count,
        tag_count,
        link_count,
        image_count: total_image_count,
        avg_word_count,
        vault_size_bytes,
        total_outgoing_links: link_count,
        total_backlinks: link_count,
        top_tags,
        most_linked,
        activity_by_month,
        notes_per_folder,
        longest_notes,
        shortest_notes,
        orphan_notes,
        no_tag_notes,
        modified_this_week: NoteList {
            count: week_total,
            notes: week_notes,
        },
        modified_this_month: NoteList {
            count: month_total,
            notes: month_notes,
        },
    }
}

/// The zero-padded `YYYY-MM` UTC month a nanosecond Unix timestamp falls in,
/// matching `strftime('%Y-%m', mtime_ns / 1e9, 'unixepoch')`'s UTC bucketing.
fn month_key(mtime_ns: i64) -> String {
    let seconds = mtime_ns.div_euclid(1_000_000_000);
    let nanos = mtime_ns.rem_euclid(1_000_000_000) as u32;
    chrono::DateTime::from_timestamp(seconds, nanos)
        .map(|datetime| datetime.format("%Y-%m").to_string())
        .unwrap_or_default()
}

/// The first path segment of a note's Vault-relative path, or `""` for a
/// Vault-root note — matching the legacy SQL's
/// `instr(relative_path, '/')`-based top-level folder split.
fn top_folder(relative_path: &str) -> String {
    relative_path
        .split_once('/')
        .map_or(String::new(), |(folder, _rest)| folder.to_string())
}

fn word_count_for_content(content: &str) -> usize {
    strip_frontmatter(content).split_whitespace().count()
}

fn strip_frontmatter(content: &str) -> &str {
    let stripped = content.trim_start_matches('\n');
    let Some(body) = stripped.strip_prefix("---\n") else {
        return content;
    };
    if let Some(pos) = body.find("\n---\n") {
        return &body[pos + 5..];
    }
    if body.strip_suffix("\n---").is_some() {
        return "";
    }
    content
}

#[cfg(test)]
mod parsing_tests {
    use super::*;

    #[test]
    fn vault_scope_parses_all_and_canonical_ids_and_rejects_everything_else() {
        assert_eq!(
            VaultScope::parse("all").expect("all parses"),
            VaultScope::All
        );

        let vault_id = VaultId::generate().expect("generate Vault id");
        assert_eq!(
            VaultScope::parse(&vault_id.to_string()).expect("uuid parses"),
            VaultScope::One(vault_id)
        );

        let error = VaultScope::parse("not-a-scope").expect_err("malformed scope rejected");
        assert_eq!(error.code, "invalid_scope");
        assert!(!error.retryable);
        assert_eq!(error.vault_id, None);

        let error = VaultScope::parse("All").expect_err("case-sensitive literal only");
        assert_eq!(error.code, "invalid_scope");
    }

    #[test]
    fn layer_selection_matches_default_all_and_named_token_semantics() {
        let surface = BrowseSurface::Everything;
        assert_eq!(
            surface.layer_selection(None),
            LayerSelection::default_surface()
        );
        assert_eq!(
            surface.layer_selection(Some("")),
            LayerSelection::default_surface()
        );
        assert_eq!(surface.layer_selection(Some("all")), LayerSelection::All);
        assert_eq!(surface.layer_selection(Some("ALL")), LayerSelection::All);

        assert_eq!(
            surface.layer_selection(Some("sources")),
            LayerSelection::Set {
                include_default: false,
                layers: ["sources".to_string()].into_iter().collect(),
            }
        );
        assert_eq!(
            surface.layer_selection(Some("default, sources")),
            LayerSelection::Set {
                include_default: true,
                layers: ["sources".to_string()].into_iter().collect(),
            }
        );
    }

    /// #109: on a restricted surface the selector is not an escape hatch, and
    /// it is clamped rather than rejected, so a `layers=` link keeps working
    /// and returns what an unadorned read would.
    #[test]
    fn a_restricted_surface_clamps_every_selector_to_the_default_surface() {
        for raw in [None, Some("all"), Some("sources"), Some("default,sources")] {
            assert_eq!(
                BrowseSurface::DefaultOnly.layer_selection(raw),
                LayerSelection::default_surface(),
                "{raw:?}"
            );
        }
    }

    /// A code neither surface knows must not leak as a novel one, and the
    /// core's internal spelling for a missing runtime must not either.
    #[test]
    fn public_read_error_codes_collapse_unknown_and_internal_spellings() {
        let vault_id = VaultId::generate().expect("generate Vault id");
        let error = unavailable(
            vault_id,
            "vault_runtime_not_active",
            "no runtime".to_string(),
            true,
        );
        assert_eq!(error.public_code(), "vault_unavailable");

        let error = unavailable(vault_id, "something_new", "?".to_string(), false);
        let operation = error.into_operation_error();
        assert_eq!(operation.code, "vault_unavailable");
        assert_eq!(operation.vault_id, Some(vault_id));

        let error = unavailable(vault_id, "vault_disabled", "off".to_string(), false);
        assert_eq!(error.into_operation_error().code, "vault_disabled");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
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
                        archive_folder: None,
                        commit_identity: None,
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
    fn vault_scope_serializes_as_the_flat_scalar_the_wire_contract_documents() {
        let vault_id = VaultId::generate().expect("generate Vault id");
        assert_eq!(
            serde_json::to_string(&VaultScope::One(vault_id)).expect("serialize one"),
            format!("\"{vault_id}\"")
        );
        assert_eq!(
            serde_json::to_string(&VaultScope::All).expect("serialize all"),
            "\"all\""
        );
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

    #[test]
    fn resolve_batch_resolves_asset_targets_by_name_alongside_note_targets() {
        let workspace = workspace(&[(
            "First",
            &[
                ("97_Notes/Some note.md", "![[Some document.pdf]] [[Shared]]"),
                ("Shared.md", "# Shared"),
                ("98_Attachments/Some document.pdf", "pdf"),
            ],
        )]);
        let reads = VaultReadCore::new(&workspace.cache, &workspace.vaults);
        let first = workspace.vault_ids[0];

        let (notes, assets) = reads
            .resolve_batch(
                first,
                &["Shared".to_string()],
                &["Some document.pdf".to_string(), "Absent.png".to_string()],
                "97_Notes",
            )
            .expect("resolve batch");

        assert_eq!(notes[0].as_ref().expect("shared resolved").slug, "shared");
        assert_eq!(
            assets[0].as_deref(),
            Some("98_Attachments/Some document.pdf")
        );
        assert_eq!(assets[1], None);
    }

    #[test]
    fn resolve_wikilinks_batch_resolves_every_target_and_matches_the_single_call() {
        let workspace = workspace(&[(
            "First",
            &[
                ("Home.md", "# Home\n\n[[Shared]]"),
                ("Shared.md", "# Shared"),
            ],
        )]);
        let reads = VaultReadCore::new(&workspace.cache, &workspace.vaults);
        let first = workspace.vault_ids[0];

        let targets = vec!["Shared".to_string(), "Missing".to_string()];
        let batch = reads
            .resolve_wikilinks(first, &targets)
            .expect("batch resolve");
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].as_ref().expect("shared resolved").slug, "shared");
        assert!(batch[1].is_none());

        let single = reads
            .resolve_wikilink(first, "Shared")
            .expect("single resolve")
            .expect("shared");
        assert_eq!(batch[0].as_ref().unwrap().slug, single.slug);
    }

    #[test]
    fn vault_directory_resolves_the_enabled_vaults_own_path_and_gates_like_exact_reads() {
        let workspace = workspace(&[
            ("First", &[("Home.md", "# Home")]),
            ("Second", &[("Home.md", "# Home")]),
        ]);
        let reads = VaultReadCore::new(&workspace.cache, &workspace.vaults);
        let first = workspace.vault_ids[0];

        let directory = reads.vault_directory(first).expect("first Vault directory");
        assert_eq!(
            std::fs::canonicalize(&directory).expect("canonical directory"),
            std::fs::canonicalize(&workspace.vault_paths[0]).expect("canonical vault root")
        );

        let missing_id = crate::vault_registry::VaultId::generate().expect("generate Vault id");
        let missing = reads
            .vault_directory(missing_id)
            .expect_err("unknown Vault id");
        assert_eq!(missing.code, "vault_not_found");
    }

    #[test]
    fn vault_directory_reports_a_retryable_error_when_the_local_directory_is_missing() {
        // A managed-Git Vault can be enabled and accepting operations before
        // its checkout has materialized on disk; the directory check must
        // report the same retryable code an exact-note read's index build
        // would, not an unrelated non-retryable filesystem error surfaced
        // later by a caller that only checked existence indirectly.
        let workspace = workspace(&[("First", &[("Home.md", "# Home")])]);
        let reads = VaultReadCore::new(&workspace.cache, &workspace.vaults);
        let first = workspace.vault_ids[0];
        std::fs::remove_dir_all(&workspace.vault_paths[0]).expect("remove vault directory");

        let error = reads
            .vault_directory(first)
            .expect_err("missing local directory");
        assert_eq!(error.code, "vault_read_unavailable");
        assert!(error.retryable);
    }

    #[test]
    fn exact_note_for_download_returns_the_note_and_directory_from_one_control_block_fetch() {
        let workspace = workspace(&[("First", &[("Home.md", "# Home\n\nfirst")])]);
        let reads = VaultReadCore::new(&workspace.cache, &workspace.vaults);
        let first = workspace.vault_ids[0];

        let (note, directory) = reads
            .exact_note_for_download(first, "home")
            .expect("lookup")
            .expect("home found");
        assert_eq!(note.vault_id, first);
        assert!(note.note.content.contains("first"));
        assert_eq!(
            std::fs::canonicalize(&directory).expect("canonical directory"),
            std::fs::canonicalize(&workspace.vault_paths[0]).expect("canonical vault root")
        );

        let missing = reads
            .exact_note_for_download(first, "does-not-exist")
            .expect("lookup succeeds");
        assert!(missing.is_none());
    }

    #[test]
    fn statistics_detail_computes_the_rich_report_from_one_vaults_snapshot() {
        let workspace = workspace(&[(
            "First",
            &[
                (
                    "Home.md",
                    "---\ntags: [alpha]\n---\n# Home\n\nfirst second third\n\n[[Second]]",
                ),
                (
                    "Second.md",
                    "---\ntags: [alpha, beta]\n---\n# Second\n\nfourth fifth",
                ),
                ("Folder/Orphan.md", "# Orphan\n\nlonely"),
            ],
        )]);
        let reads = VaultReadCore::new(&workspace.cache, &workspace.vaults);
        let first = workspace.vault_ids[0];

        let result = reads
            .statistics_detail(first)
            .expect("statistics detail succeeds");
        assert_eq!(result.vault_id, first);
        let stats = result.stats;

        assert_eq!(stats.note_count, 3);
        assert_eq!(stats.tag_count, 2);
        assert_eq!(stats.link_count, 1);
        assert_eq!(stats.total_outgoing_links, 1);
        assert_eq!(stats.total_backlinks, 1);
        assert_eq!(
            stats
                .top_tags
                .iter()
                .map(|tag| (tag.tag.as_str(), tag.note_count))
                .collect::<Vec<_>>(),
            vec![("alpha", 2), ("beta", 1)]
        );
        assert_eq!(
            stats
                .most_linked
                .iter()
                .map(|note| (note.slug.as_str(), note.backlink_count))
                .collect::<Vec<_>>(),
            vec![("second", 1)]
        );
        assert_eq!(
            stats
                .orphan_notes
                .iter()
                .map(|note| note.slug.as_str())
                .collect::<Vec<_>>(),
            vec!["orphan"]
        );
        assert_eq!(
            stats
                .no_tag_notes
                .iter()
                .map(|note| note.slug.as_str())
                .collect::<Vec<_>>(),
            vec!["orphan"]
        );
        let folder_counts: BTreeMap<&str, i64> = stats
            .notes_per_folder
            .iter()
            .map(|folder| (folder.folder.as_str(), folder.note_count))
            .collect();
        assert_eq!(folder_counts.get(""), Some(&2));
        assert_eq!(folder_counts.get("Folder"), Some(&1));
    }

    #[test]
    fn statistics_detail_reports_vault_not_found_for_an_unregistered_vault() {
        let workspace = workspace(&[("First", &[("Home.md", "# Home\n")])]);
        let reads = VaultReadCore::new(&workspace.cache, &workspace.vaults);

        let missing = VaultId::generate().expect("generate Vault id");
        let error = reads
            .statistics_detail(missing)
            .expect_err("unregistered Vault errs");
        assert_eq!(error.code, "vault_not_found");
        assert!(!error.retryable);
    }
}
