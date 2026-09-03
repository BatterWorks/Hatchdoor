//! Shared search vocabulary: modes, layer selection, and the outbound-link
//! shape the Vault-qualified core builds its responses from.

use schemars::JsonSchema;

use serde::{Deserialize, Serialize};

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
