//! Phase 2 search orchestrator. Consumed by both MCP and HTTP.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::cache::SqliteCache;
use crate::embed::Embedder;
use crate::vault::{NoteMetadata, NoteSummary};

pub mod assemble;
pub mod retrieve;

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
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NoteFilters {
    pub tags: Vec<String>,
    pub path_prefix: Option<String>,
    pub property_exists: Vec<String>,
    pub property_equals: BTreeMap<String, serde_json::Value>,
}

impl NoteFilters {
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
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
    let hits = retrieve::retrieve(cache, embedder, &req)?;
    let results = assemble::assemble(cache, hits, &req.include_properties)?;
    Ok(SearchResponse { mode, results })
}

pub fn query_notes(
    cache: &SqliteCache,
    filters: &NoteFilters,
    include_properties: &[String],
    limit: usize,
) -> Result<Vec<NoteSummary>, String> {
    Ok(cache
        .note_summaries()?
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
) -> Result<Option<HashSet<String>>, String> {
    if filters.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        cache
            .note_summaries()?
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

    use super::{NoteFilters, SearchMode, SearchRequest, run};

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
                    path_prefix: Some("Devices".to_string()),
                    property_exists: vec!["status".to_string()],
                    property_equals: serde_json::from_value(serde_json::json!({
                        "status": "active"
                    }))
                    .expect("property filters"),
                },
                include_properties: vec!["status".to_string()],
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
            },
        )
        .expect("filtered search");

        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].note_slug, "wanted");
    }
}
