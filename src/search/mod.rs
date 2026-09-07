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

/// One tag written the way the indexer stores it: no leading `#`, no
/// surrounding `/`, lower-cased. `None` when nothing is left.
///
/// `pub(crate)` because a metadata query selects by tag too
/// (`vault_read::query`), and a query that normalised tags differently from
/// the indexer would answer a caller's `#Space/Hobby` with nothing.
pub(crate) fn normalize_tag_path(raw: &str) -> Option<String> {
    let normalized = raw
        .trim()
        .trim_start_matches('#')
        .trim_matches('/')
        .to_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

/// Whether a Note's stored tag satisfies a query for `wanted`: the tag itself,
/// or any tag nested under it. `topic` matches `topic` and `topic/sub`, and
/// never `topical`.
///
/// Both arguments are already normalised by [`normalize_tag_path`].
///
/// `vault_scoped::tag_results` applies this same predicate inline to the `#tag`
/// search shorthand and deliberately still does: it sits on the retrieval path,
/// where a rewrite costs an eval run (ADR-15), and this change had no reason to
/// spend one. So the two copies are held in step by whoever edits either, not
/// by construction — change this one and the `tag_results` copy is the other
/// place to look.
pub(crate) fn tag_matches(candidate: &str, wanted: &str) -> bool {
    candidate == wanted
        || candidate
            .strip_prefix(wanted)
            .is_some_and(|tail| tail.starts_with('/'))
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
