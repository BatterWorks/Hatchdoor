//! Shared search vocabulary: modes, note filters, layer selection, and the
//! metadata projection the Vault-qualified core builds its responses from.

use schemars::JsonSchema;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::vault::{NoteMetadata, NoteSummary};

pub mod layer_selection;
pub mod vault_scoped;

pub use layer_selection::{LayerInfo, LayerSelection};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    #[default]
    Semantic,
    Keyword,
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

#[derive(Debug, Clone, Serialize, JsonSchema, Deserialize)]
pub struct OutboundLink {
    pub slug: String,
    pub title: String,
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
