//! Phase 2 search orchestrator. Consumed by both MCP and HTTP.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::cache::SqliteCache;
use crate::embed::Embedder;
use crate::vault::{NoteMetadata, NoteSummary};

pub mod assemble;
pub mod layer_selection;
pub mod retrieve;
pub mod vault_scoped;

pub use layer_selection::{LayerInfo, LayerSelection};
pub use retrieve::ChunkHit;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    #[default]
    Semantic,
    Keyword,
}

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub query: String,
    pub mode: SearchMode,
    pub limit: usize,
    pub per_note_cap: usize,
    pub filters: NoteFilters,
    pub include_properties: Vec<String>,
    /// Which layers to search. Defaults to the default surface only; Group D
    /// wires the MCP `layers` parameter through to here.
    pub layers: LayerSelection,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NoteFilters {
    pub tags: Vec<String>,
    pub tag_prefixes: Vec<String>,
    pub path_prefix: Option<String>,
    pub property_exists: Vec<String>,
    pub property_equals: BTreeMap<String, serde_json::Value>,
}

impl NoteFilters {
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
            && self.tag_prefixes.is_empty()
            && self.path_prefix.as_deref().is_none_or(str::is_empty)
            && self.property_exists.is_empty()
            && self.property_equals.is_empty()
    }

    fn matches(&self, note: &NoteSummary) -> bool {
        let normalized_tags = self
            .tags
            .iter()
            .map(|tag| tag.trim().trim_start_matches('#').to_lowercase())
            .collect::<Vec<_>>();
        if !normalized_tags
            .iter()
            .all(|tag| note.metadata.tags.contains(tag))
        {
            return false;
        }
        let normalized_tag_prefixes = self
            .tag_prefixes
            .iter()
            .filter_map(|tag| normalize_tag_path(tag))
            .collect::<Vec<_>>();
        if normalized_tag_prefixes.len() != self.tag_prefixes.len()
            || !normalized_tag_prefixes.iter().all(|prefix| {
                note.metadata.tags.iter().any(|tag| {
                    tag == prefix
                        || tag
                            .strip_prefix(prefix)
                            .is_some_and(|rest| rest.starts_with('/'))
                })
            })
        {
            return false;
        }
        if let Some(prefix) = self.path_prefix.as_deref()
            && !note
                .relative_path
                .to_lowercase()
                .starts_with(&prefix.trim().trim_matches('/').to_lowercase())
        {
            return false;
        }
        let Some(properties) = note.metadata.properties.as_object() else {
            return self.property_exists.is_empty() && self.property_equals.is_empty();
        };
        self.property_exists
            .iter()
            .all(|key| properties.contains_key(key))
            && self
                .property_equals
                .iter()
                .all(|(key, value)| properties.get(key) == Some(value))
    }
}

fn normalize_tag_path(raw: &str) -> Option<String> {
    let normalized = raw
        .trim()
        .trim_start_matches('#')
        .trim_matches('/')
        .to_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn tag_prefix_query(query: &str) -> Option<String> {
    let tag = query.trim().strip_prefix('#')?;
    if tag.is_empty()
        || !tag
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '-' | '_' | '/'))
    {
        return None;
    }
    normalize_tag_path(tag)
}

#[derive(Debug, Clone, Serialize)]
pub struct OutboundLink {
    pub slug: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub chunk_id: i64,
    pub note_slug: String,
    pub note_title: String,
    pub note_path: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub score: f32,
    /// The hit note's layer (`None` = default surface).
    pub layer: Option<String>,
    pub outbound_links: Vec<OutboundLink>,
    pub metadata: NoteMetadata,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub mode: SearchMode,
    pub results: Vec<SearchResult>,
}

pub fn run(
    cache: &SqliteCache,
    embedder: &dyn Embedder,
    req: SearchRequest,
) -> Result<SearchResponse, String> {
    let trimmed = req.query.trim();
    if trimmed.is_empty() {
        return Err("query cannot be empty".to_string());
    }
    let req = SearchRequest {
        query: trimmed.to_string(),
        ..req
    };
    let mode = req.mode;
    if let Some(tag_prefix) = tag_prefix_query(&req.query) {
        let mut filters = req.filters;
        filters.tag_prefixes.push(tag_prefix.clone());
        let results = query_notes(
            cache,
            &filters,
            &req.include_properties,
            req.limit,
            &req.layers,
        )?
        .into_iter()
        .map(|note| SearchResult {
            chunk_id: 0,
            note_slug: note.slug,
            note_title: note.title,
            note_path: note.relative_path,
            heading_path: None,
            content: format!("Matched tag: #{tag_prefix}"),
            score: 1.0,
            layer: note.layer,
            outbound_links: Vec::new(),
            metadata: note.metadata,
        })
        .collect();
        return Ok(SearchResponse { mode, results });
    }
    let hits = retrieve::retrieve(cache, embedder, &req)?;
    let results = assemble::assemble(cache, hits, &req.include_properties)?;
    Ok(SearchResponse { mode, results })
}

pub fn query_notes(
    cache: &SqliteCache,
    filters: &NoteFilters,
    include_properties: &[String],
    limit: usize,
    layers: &LayerSelection,
) -> Result<Vec<NoteSummary>, String> {
    // Honor the caller's layer selection: an omitted/default selection returns
    // the default surface only, so demoted notes never leak from query_notes.
    Ok(cache
        .note_summaries(layers)?
        .into_iter()
        .filter(|note| filters.matches(note))
        .take(limit)
        .map(|mut note| {
            note.metadata = project_metadata(&note.metadata, include_properties);
            note
        })
        .collect())
}

pub(crate) fn matching_note_slugs(
    cache: &SqliteCache,
    filters: &NoteFilters,
    layers: &LayerSelection,
) -> Result<Option<HashSet<String>>, String> {
    if filters.is_empty() {
        return Ok(None);
    }
    // Scope the eligible set to the same selection the search itself uses, so a
    // metadata pre-filter never widens the layer scope of a search.
    Ok(Some(
        cache
            .note_summaries(layers)?
            .into_iter()
            .filter(|note| filters.matches(note))
            .map(|note| note.slug)
            .collect(),
    ))
}

pub(crate) fn project_metadata(
    metadata: &NoteMetadata,
    include_properties: &[String],
) -> NoteMetadata {
    let properties = metadata.properties.as_object();
    let selected = include_properties
        .iter()
        .filter_map(|key| {
            properties
                .and_then(|map| map.get(key))
                .map(|value| (key, value))
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    NoteMetadata {
        tags: metadata.tags.clone(),
        aliases: metadata.aliases.clone(),
        properties: serde_json::Value::Object(selected),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::vault::VaultIndex;

    use super::{NoteFilters, SearchMode, SearchRequest, query_notes, run};

    fn build_cache(files: &[(&str, &str)]) -> (SqliteCache, Arc<dyn Embedder>) {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create fixture directory");
            }
            std::fs::write(path, body).expect("write");
        }
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");
        (cache, embedder)
    }

    #[test]
    fn semantic_path_end_to_end() {
        let (cache, embedder) = build_cache(&[
            ("Alpha.md", "# Alpha\n\napples and oranges"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let resp = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "apples".to_string(),
                mode: SearchMode::Semantic,
                limit: 10,
                per_note_cap: 2,
                filters: Default::default(),
                include_properties: Vec::new(),
                layers: crate::search::LayerSelection::default_surface(),
            },
        )
        .expect("run");
        assert_eq!(resp.mode, SearchMode::Semantic);
        assert!(!resp.results.is_empty());
        assert!(resp.results[0].note_title == "Alpha" || resp.results[0].note_title == "Bravo");
    }

    #[test]
    fn keyword_path_end_to_end() {
        let (cache, embedder) = build_cache(&[
            ("Alpha.md", "# Alpha\n\noranges"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let resp = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "oranges".to_string(),
                mode: SearchMode::Keyword,
                limit: 10,
                per_note_cap: 2,
                filters: Default::default(),
                include_properties: Vec::new(),
                layers: crate::search::LayerSelection::default_surface(),
            },
        )
        .expect("run");
        assert_eq!(resp.mode, SearchMode::Keyword);
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].note_slug, "alpha");
    }

    #[test]
    fn empty_query_errors() {
        let (cache, embedder) = build_cache(&[("Alpha.md", "# Alpha\n\nbody")]);
        let err = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "   ".to_string(),
                mode: SearchMode::Semantic,
                limit: 10,
                per_note_cap: 2,
                filters: Default::default(),
                include_properties: Vec::new(),
                layers: crate::search::LayerSelection::default_surface(),
            },
        )
        .expect_err("expected empty-query error");
        assert!(err.to_lowercase().contains("empty"));
    }

    #[test]
    fn over_fetch_compensates_for_single_note_flooding() {
        // One note with many distinct chunks (heading-separated). per_note_cap=1 means
        // only one chunk from this note can appear, but limit=3 should still try.
        let body = (0..20)
            .map(|i| format!("# H{i}\n\nsection {i} body text"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let (cache, embedder) = build_cache(&[
            ("Alpha.md", body.as_str()),
            ("Bravo.md", "# Bravo\n\nunrelated"),
        ]);
        let resp = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "section".to_string(),
                mode: SearchMode::Keyword,
                limit: 3,
                per_note_cap: 1,
                filters: Default::default(),
                include_properties: Vec::new(),
                layers: crate::search::LayerSelection::default_surface(),
            },
        )
        .expect("run");
        // With per_note_cap=1, at most 1 chunk from Alpha. We may get 1 from Alpha + 0..1 from Bravo.
        let alpha_count = resp
            .results
            .iter()
            .filter(|r| r.note_slug == "alpha")
            .count();
        assert!(alpha_count <= 1);
    }

    #[test]
    fn metadata_filters_constrain_search_and_properties_are_projected() {
        let (cache, embedder) = build_cache(&[
            (
                "Devices/Router.md",
                "---\ntags: [type/device, action/review]\naliases: [Gateway]\nstatus: active\nprivate: hidden\n---\n# Router\n\nrouter network",
            ),
            (
                "Archive/Old Router.md",
                "---\ntags: [type/device]\nstatus: retired\n---\n# Old Router\n\nrouter network",
            ),
        ]);
        let response = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "router".to_string(),
                mode: SearchMode::Keyword,
                limit: 10,
                per_note_cap: 2,
                filters: NoteFilters {
                    tags: vec!["ACTION/REVIEW".to_string()],
                    tag_prefixes: Vec::new(),
                    path_prefix: Some("Devices".to_string()),
                    property_exists: vec!["status".to_string()],
                    property_equals: serde_json::from_value(serde_json::json!({
                        "status": "active"
                    }))
                    .expect("property filters"),
                },
                include_properties: vec!["status".to_string()],
                layers: crate::search::LayerSelection::default_surface(),
            },
        )
        .expect("filtered search");

        assert_eq!(response.results.len(), 1);
        let result = &response.results[0];
        assert_eq!(result.note_slug, "router");
        assert_eq!(result.metadata.tags, vec!["action/review", "type/device"]);
        assert_eq!(result.metadata.aliases, vec!["Gateway"]);
        assert_eq!(
            result.metadata.properties,
            serde_json::json!({"status":"active"})
        );

        let semantic = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "network gateway".to_string(),
                mode: SearchMode::Semantic,
                limit: 10,
                per_note_cap: 2,
                filters: NoteFilters {
                    tags: vec!["action/review".to_string()],
                    ..Default::default()
                },
                include_properties: Vec::new(),
                layers: crate::search::LayerSelection::default_surface(),
            },
        )
        .expect("filtered semantic search");
        assert!(!semantic.results.is_empty());
        assert!(
            semantic
                .results
                .iter()
                .all(|result| result.note_slug == "router")
        );
    }

    #[test]
    fn nested_tag_query_returns_the_parent_and_all_descendants() {
        let (cache, embedder) = build_cache(&[
            (
                "Selfhosting.md",
                "---\ntags: [topic/selfhosting]\n---\n# Self-hosting\n\nA parent tag note.",
            ),
            (
                "Immich.md",
                "---\ntags: [topic/selfhosting/immich]\n---\n# Immich\n\nPhoto management.",
            ),
            (
                "OpenCloud.md",
                "---\ntags: [topic/selfhosting/opencloud]\n---\n# OpenCloud\n\nFile sharing.",
            ),
            (
                "Not A Child.md",
                "---\ntags: [topic/selfhostingish/other]\n---\n# Not a child\n\nDifferent tag branch.",
            ),
        ]);

        let response = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "#Topic/SelfHosting".to_string(),
                mode: SearchMode::Semantic,
                limit: 10,
                per_note_cap: 2,
                filters: Default::default(),
                include_properties: Vec::new(),
                layers: crate::search::LayerSelection::default_surface(),
            },
        )
        .expect("nested tag search");

        let slugs = response
            .results
            .into_iter()
            .map(|result| result.note_slug)
            .collect::<Vec<_>>();
        assert_eq!(slugs, vec!["immich", "opencloud", "selfhosting"]);
    }

    #[test]
    fn tag_prefix_filters_match_only_a_tag_branch() {
        let (cache, _embedder) = build_cache(&[
            (
                "Immich.md",
                "---\ntags: [topic/selfhosting/immich]\n---\n# Immich",
            ),
            (
                "OpenCloud.md",
                "---\ntags: [topic/selfhosting/opencloud]\n---\n# OpenCloud",
            ),
            (
                "Different.md",
                "---\ntags: [topic/selfhostingish/other]\n---\n# Different",
            ),
        ]);

        let notes = query_notes(
            &cache,
            &NoteFilters {
                tag_prefixes: vec!["#TOPIC/SELFHOSTING".to_string()],
                ..Default::default()
            },
            &[],
            10,
            &crate::search::LayerSelection::default_surface(),
        )
        .expect("query tag branch");

        assert_eq!(
            notes.into_iter().map(|note| note.slug).collect::<Vec<_>>(),
            vec!["immich", "opencloud"]
        );
    }

    #[test]
    fn query_notes_honors_layer_selection() {
        // The MCP `query_notes` tool must not leak demoted notes. Omitted layers
        // (the default selection) returns default-surface notes only; naming the
        // layer reveals it. This is carried-forward correctness item 1: before
        // the fix, query_notes passed LayerSelection::all() and leaked.
        let (cache, _embedder) = build_cache(&[
            ("wiki/Page.md", "---\ntags: [t/x]\n---\n# Page"),
            ("sources/.hatchdoor-layer", "sources"),
            ("sources/Clip.md", "---\ntags: [t/x]\n---\n# Clip"),
        ]);
        let filters = NoteFilters {
            tags: vec!["t/x".to_string()],
            ..Default::default()
        };

        let default = query_notes(
            &cache,
            &filters,
            &[],
            10,
            &crate::search::LayerSelection::default_surface(),
        )
        .expect("default query");
        let default_slugs: Vec<String> = default.iter().map(|n| n.slug.clone()).collect();
        assert!(default_slugs.contains(&"page".to_string()));
        assert!(
            !default_slugs.contains(&"clip".to_string()),
            "query_notes must not leak demoted notes under the default selection"
        );

        let (selection, _) = crate::search::LayerSelection::parse(
            &["sources".to_string()],
            &["sources".to_string()],
        );
        let sourced = query_notes(&cache, &filters, &[], 10, &selection).expect("sourced query");
        assert!(
            sourced.iter().any(|n| n.slug == "clip"),
            "selecting the layer must reveal its demoted notes"
        );
        assert!(
            sourced.iter().all(|n| n.slug != "page"),
            "a named-layer selection returns that layer only, not the default surface"
        );
    }

    #[test]
    fn metadata_filtering_is_not_limited_to_the_global_top_two_hundred_chunks() {
        let dir = TempDir::new().expect("tempdir");
        for index in 0..205 {
            std::fs::write(
                dir.path().join(format!("Distractor {index:03}.md")),
                format!("# Distractor\n\n{}", "router ".repeat(40)),
            )
            .expect("write distractor");
        }
        std::fs::write(
            dir.path().join("Wanted.md"),
            "---\ntags: [wanted/result]\n---\n# Wanted\n\nrouter",
        )
        .expect("write wanted");
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");

        let response = run(
            &cache,
            embedder.as_ref(),
            SearchRequest {
                query: "router".to_string(),
                mode: SearchMode::Keyword,
                limit: 10,
                per_note_cap: 2,
                filters: NoteFilters {
                    tags: vec!["wanted/result".to_string()],
                    ..Default::default()
                },
                include_properties: Vec::new(),
                layers: crate::search::LayerSelection::default_surface(),
            },
        )
        .expect("filtered search");

        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].note_slug, "wanted");
    }
}
