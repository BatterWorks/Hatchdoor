//! Selecting Notes by tag, path, and frontmatter property.
//!
//! A query *selects*. A Note either satisfies every stated condition or it does
//! not, and the Notes come back in a stable order rather than ranked. That is
//! the whole distinction from search, which finds Notes by meaning and orders
//! them by how well they match, and it is why nothing here touches the
//! retrieval path: no embedding, no scoring, no candidate cap.
//!
//! Everything a condition tests — tags, the Vault-relative path, frontmatter
//! properties — is already in the published snapshot rows, so a Vault whose
//! generation carries no vectors answers a query in full.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cache::vault_snapshots::VaultSnapshotNote;
use crate::search::{normalize_tag_path, tag_matches};
use crate::vault_registry::VaultId;

use super::VaultReadError;

/// The most conditions and projected property names one query may carry.
/// Inherited from the retired `query_notes` tool: a request past this size is
/// a client bug rather than a query anyone wrote.
const MAX_QUERY_TERMS: usize = 50;

/// The longest `path_prefix` a condition may carry, matching the retired
/// tool's bound and comfortably past any real Vault path.
const MAX_PATH_PREFIX_BYTES: usize = 4_096;

/// The default and bounds a query's `limit` is held to. Applied by
/// [`CompiledQuery::compile`] rather than by each adapter, so a caller cannot
/// reach the core with a limit of zero and be told its complete answer was
/// truncated.
fn clamp_query_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(50).clamp(1, 200)
}

/// One request for the Notes whose tags, path, or properties satisfy every
/// stated condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteQuery {
    /// Conditions are ANDed: a Note qualifies only by satisfying all of them.
    /// An empty list is refused rather than treated as "every Note" — see
    /// [`CompiledQuery::compile`].
    pub conditions: Vec<NoteQueryCondition>,
    /// Frontmatter property names to project onto each returned row. Names the
    /// Note does not carry are simply absent from its row.
    pub properties: Vec<String>,
    /// Maximum rows to return. `None` takes the default; anything outside the
    /// advertised bounds clamps into them.
    pub limit: Option<usize>,
}

/// One test a Note must satisfy.
///
/// Input only, so it carries no `JsonSchema`: a tool's `inputSchema` is
/// hand-written (`query_notes_tool_schema`), and only results generate their
/// schema from the Rust type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum NoteQueryCondition {
    /// The Note carries this tag, or a tag nested under it. Matched exactly the
    /// way the `#tag` search shorthand matches: `topic` selects `topic` and
    /// `topic/sub`, never `topical`.
    Tag { tag: String },
    /// The Note lives at or under this Vault-relative folder. Segment-aware and
    /// case-insensitive: `notes` selects `notes/a.md` but never
    /// `notes-archive/a.md`.
    PathPrefix { prefix: String },
    /// The Note's frontmatter property `name` satisfies `operator`. `tags`
    /// and `aliases` are refused here — see [`compile_condition`].
    Property {
        name: String,
        operator: PropertyOperator,
        /// Required by the six comparing operators, refused by the four that
        /// test presence or emptiness.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
    },
}

/// How a property condition tests the value it finds.
///
/// Every operator but [`PropertyOperator::Missing`] requires the property to be
/// present: a Note that never mentions `status` does not have a status
/// different from `draft`, so `ne` does not select it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyOperator {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
    Exists,
    Missing,
    Empty,
    NotEmpty,
}

impl PropertyOperator {
    /// Whether this operator compares against a caller-supplied value. The four
    /// that do not must not be sent one, so a `value` a caller believed was
    /// being applied can never be silently dropped.
    fn takes_value(self) -> bool {
        matches!(
            self,
            Self::Eq | Self::Ne | Self::Lt | Self::Lte | Self::Gt | Self::Gte
        )
    }

    /// This operator's own wire spelling, so a refusal names what the caller
    /// actually sent. Spelled out rather than derived from `Serialize`, which
    /// would need a fallback for a failure that cannot happen and would put
    /// that fallback's wording in front of a caller.
    fn wire_name(self) -> &'static str {
        match self {
            Self::Eq => "eq",
            Self::Ne => "ne",
            Self::Lt => "lt",
            Self::Lte => "lte",
            Self::Gt => "gt",
            Self::Gte => "gte",
            Self::Exists => "exists",
            Self::Missing => "missing",
            Self::Empty => "empty",
            Self::NotEmpty => "not_empty",
        }
    }
}

/// One selected Note, qualified by its Vault, carrying the properties the query
/// asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteQueryRow {
    pub vault_id: VaultId,
    pub title: String,
    pub slug: String,
    pub relative_path: String,
    /// The requested property names this Note actually carries. Absent names
    /// are omitted rather than reported as null, so "no value" and "the value
    /// is null" stay distinguishable.
    pub properties: BTreeMap<String, Value>,
}

/// A query's answer, inside the shared collection envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteQueryResponse {
    pub notes: Vec<NoteQueryRow>,
    /// Whether more Notes qualified than `limit` allowed through. Reported
    /// rather than left for the caller to infer from a full page, because a
    /// selection that quietly stops short reads as a complete answer.
    pub truncated: bool,
}

/// A validated query with its conditions normalised once, ready to run against
/// any number of Vault snapshots.
#[derive(Debug)]
pub(crate) struct CompiledQuery {
    conditions: Vec<CompiledCondition>,
    properties: Vec<String>,
    pub(crate) limit: usize,
}

#[derive(Debug)]
enum CompiledCondition {
    Tag(String),
    PathPrefix(String),
    Property {
        name: String,
        operator: PropertyOperator,
        value: Option<Value>,
    },
}

impl CompiledQuery {
    /// Validate and normalise one query, before any Vault is touched, so a
    /// malformed request is refused identically whatever its scope.
    pub(crate) fn compile(query: &NoteQuery) -> Result<Self, VaultReadError> {
        if query.conditions.is_empty() {
            return Err(invalid_query(
                "a query needs at least one condition; use get_tree or recently_modified to read a Vault without selecting",
            ));
        }
        for (label, count) in [
            ("conditions", query.conditions.len()),
            ("properties", query.properties.len()),
        ] {
            if count > MAX_QUERY_TERMS {
                return Err(invalid_query(format!(
                    "{label} accepts at most {MAX_QUERY_TERMS} entries"
                )));
            }
        }
        if query.properties.iter().any(|name| name.trim().is_empty()) {
            return Err(invalid_query("a projected property name cannot be empty"));
        }

        let conditions = query
            .conditions
            .iter()
            .map(compile_condition)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            conditions,
            properties: query.properties.clone(),
            limit: clamp_query_limit(query.limit),
        })
    }

    /// The rows one Vault's published snapshot contributes. Unsorted and
    /// unlimited: ordering and the limit are decided once across every
    /// participating Vault, so an `all` query cannot depend on which Vault a
    /// Note happens to live in.
    pub(crate) fn rows_for(
        &self,
        vault_id: VaultId,
        notes: &[VaultSnapshotNote],
    ) -> Vec<NoteQueryRow> {
        notes
            .iter()
            .filter(|note| self.selects(note))
            .map(|note| NoteQueryRow {
                vault_id,
                title: note.title.clone(),
                slug: note.slug.clone(),
                relative_path: note.relative_path.clone(),
                properties: self.project(note),
            })
            .collect()
    }

    fn selects(&self, note: &VaultSnapshotNote) -> bool {
        self.conditions
            .iter()
            .all(|condition| condition.matches(note))
    }

    /// The requested properties this Note carries. `tags` and `aliases` are
    /// projected from their parsed lists: the frontmatter parser lifts both out
    /// of the property map, so a caller asking for either would otherwise get
    /// silence for a key its own note plainly has.
    fn project(&self, note: &VaultSnapshotNote) -> BTreeMap<String, Value> {
        let mut projected = BTreeMap::new();
        for name in &self.properties {
            let value = match name.as_str() {
                "tags" => Some(Value::from(note.metadata.tags.clone())),
                "aliases" => Some(Value::from(note.metadata.aliases.clone())),
                _ => property(note, name).cloned(),
            };
            if let Some(value) = value {
                projected.insert(name.clone(), value);
            }
        }
        projected
    }
}

impl CompiledCondition {
    fn matches(&self, note: &VaultSnapshotNote) -> bool {
        match self {
            Self::Tag(wanted) => note
                .metadata
                .tags
                .iter()
                .any(|candidate| tag_matches(candidate, wanted)),
            Self::PathPrefix(prefix) => path_under(&note.relative_path, prefix),
            Self::Property {
                name,
                operator,
                value,
            } => property_matches(property(note, name), *operator, value.as_ref()),
        }
    }
}

fn compile_condition(condition: &NoteQueryCondition) -> Result<CompiledCondition, VaultReadError> {
    match condition {
        NoteQueryCondition::Tag { tag } => normalize_tag_path(tag)
            .map(CompiledCondition::Tag)
            .ok_or_else(|| invalid_query("a tag condition cannot be empty")),
        NoteQueryCondition::PathPrefix { prefix } => {
            if prefix.len() > MAX_PATH_PREFIX_BYTES {
                return Err(invalid_query(format!(
                    "path_prefix cannot exceed {MAX_PATH_PREFIX_BYTES} bytes"
                )));
            }
            normalize_path_prefix(prefix)
                .map(CompiledCondition::PathPrefix)
                .ok_or_else(|| invalid_query("a path_prefix condition cannot be empty"))
        }
        NoteQueryCondition::Property {
            name,
            operator,
            value,
        } => {
            if name.trim().is_empty() {
                return Err(invalid_query("a property condition needs a name"));
            }
            // The frontmatter parser lifts `tags` and `aliases` out of the
            // property map into their own parsed lists, so a property
            // condition naming either would find nothing and answer `missing`
            // for every Note in the Vault — a wrong answer that looks like a
            // real one. Refuse instead, and say where the tag axis lives.
            if let Some(refusal) = match name.as_str() {
                "tags" => Some(
                    "tags is not a frontmatter property here; select by tag with a tag condition",
                ),
                "aliases" => Some(
                    "aliases is not a frontmatter property here; it can be projected but not queried",
                ),
                _ => None,
            } {
                return Err(invalid_query(refusal));
            }
            match (operator.takes_value(), value) {
                (true, None) => Err(invalid_query(format!(
                    "the {} operator needs a value to compare against",
                    operator.wire_name()
                ))),
                (false, Some(_)) => Err(invalid_query(format!(
                    "the {} operator takes no value",
                    operator.wire_name()
                ))),
                _ => Ok(CompiledCondition::Property {
                    name: name.clone(),
                    operator: *operator,
                    value: value.clone(),
                }),
            }
        }
    }
}

fn invalid_query(message: impl Into<String>) -> VaultReadError {
    VaultReadError {
        code: "invalid_query".to_string(),
        message: message.into(),
        vault_id: None,
        retryable: false,
    }
}

/// A folder path with its decoration removed and folded for case-insensitive
/// comparison, matching how `get_tree`'s `folder` argument is matched.
fn normalize_path_prefix(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .replace('\\', "/")
        .trim_matches('/')
        .to_lowercase();
    let normalized = trimmed
        .strip_prefix("./")
        .map(ToOwned::to_owned)
        .unwrap_or(trimmed);
    (!normalized.is_empty()).then_some(normalized)
}

/// Whether a Note's Vault-relative path sits at or under `prefix`. Segment
/// aware on purpose: a plain string prefix would put `notes-archive/a.md`
/// inside `notes`, and a caller reading that result has no way to tell.
fn path_under(relative_path: &str, prefix: &str) -> bool {
    let path = relative_path.to_lowercase();
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn property<'a>(note: &'a VaultSnapshotNote, name: &str) -> Option<&'a Value> {
    note.metadata.properties.as_object()?.get(name)
}

fn property_matches(
    actual: Option<&Value>,
    operator: PropertyOperator,
    expected: Option<&Value>,
) -> bool {
    let Some(actual) = actual else {
        // Absence answers exactly one question, and answers it yes.
        return operator == PropertyOperator::Missing;
    };
    match operator {
        PropertyOperator::Missing => false,
        PropertyOperator::Exists => true,
        PropertyOperator::Empty => is_empty(actual),
        PropertyOperator::NotEmpty => !is_empty(actual),
        PropertyOperator::Eq => equals(actual, expected),
        PropertyOperator::Ne => !equals(actual, expected),
        PropertyOperator::Lt => ordered(actual, expected, |ordering| ordering == Ordering::Less),
        PropertyOperator::Lte => {
            ordered(actual, expected, |ordering| ordering != Ordering::Greater)
        }
        PropertyOperator::Gt => ordered(actual, expected, |ordering| ordering == Ordering::Greater),
        PropertyOperator::Gte => ordered(actual, expected, |ordering| ordering != Ordering::Less),
    }
}

/// A present property counts as empty when it holds nothing a reader would see:
/// YAML's bare `key:` (null), a blank string, an empty list, an empty mapping.
fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(entries) => entries.is_empty(),
        _ => false,
    }
}

/// Equality, with the one accommodation a Vault forces: a multi-value property
/// (`for: [reference, archive]`) is equal to a scalar the list contains, which
/// is what an author means by "notes where `for` is `reference`".
fn equals(actual: &Value, expected: Option<&Value>) -> bool {
    let Some(expected) = expected else {
        return false;
    };
    match (actual, expected) {
        (Value::Array(items), expected) if !expected.is_array() => {
            items.iter().any(|item| scalar_equals(item, expected))
        }
        _ => scalar_equals(actual, expected),
    }
}

/// `compare` first, so YAML's `1` and a caller's `1.0` are the same number, and
/// structural equality only where there is no ordering to appeal to (null,
/// lists, mappings, and mismatched types).
fn scalar_equals(actual: &Value, expected: &Value) -> bool {
    match compare(actual, expected) {
        Some(ordering) => ordering == Ordering::Equal,
        None => actual == expected,
    }
}

/// An ordered comparison holds only between two comparable scalars. A list has
/// no position relative to a scalar and a string none relative to a number, so
/// either selects nothing rather than guessing.
fn ordered(actual: &Value, expected: Option<&Value>, accept: impl Fn(Ordering) -> bool) -> bool {
    expected
        .and_then(|expected| compare(actual, expected))
        .is_some_and(accept)
}

/// Numbers compare numerically, strings byte-wise — which orders ISO-8601 dates
/// and timestamps correctly, the form YAML frontmatter dates are stored in.
/// Anything else has no order.
fn compare(left: &Value, right: &Value) -> Option<Ordering> {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => left.as_f64()?.partial_cmp(&right.as_f64()?),
        (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
        (Value::Bool(left), Value::Bool(right)) => Some(left.cmp(right)),
        _ => None,
    }
}

/// The one order a query's results come back in, across every participating
/// Vault: by path, then by Vault, then by slug. Identical to the `#tag` search
/// shorthand's ordering, and a total order over the collection, so two
/// identical calls cannot disagree about which rows a `limit` keeps.
pub(crate) fn sort_rows(rows: &mut [NoteQueryRow]) {
    rows.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| left.vault_id.cmp(&right.vault_id))
            .then_with(|| left.slug.cmp(&right.slug))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn query(conditions: Vec<NoteQueryCondition>) -> NoteQuery {
        NoteQuery {
            conditions,
            properties: Vec::new(),
            limit: None,
        }
    }

    fn tag(tag: &str) -> NoteQueryCondition {
        NoteQueryCondition::Tag {
            tag: tag.to_string(),
        }
    }

    fn property_condition(
        name: &str,
        operator: PropertyOperator,
        value: Option<Value>,
    ) -> NoteQueryCondition {
        NoteQueryCondition::Property {
            name: name.to_string(),
            operator,
            value,
        }
    }

    #[test]
    fn a_query_with_no_conditions_is_refused_rather_than_selecting_everything() {
        let error = CompiledQuery::compile(&query(Vec::new())).expect_err("refused");
        assert_eq!(error.code, "invalid_query");
        assert!(error.message.contains("at least one condition"));
        assert!(!error.retryable);
    }

    #[test]
    fn a_comparing_operator_without_a_value_is_refused_by_name() {
        let error = CompiledQuery::compile(&query(vec![property_condition(
            "review-date",
            PropertyOperator::Lt,
            None,
        )]))
        .expect_err("refused");
        assert_eq!(error.code, "invalid_query");
        assert!(error.message.contains("lt"), "{}", error.message);
    }

    #[test]
    fn a_presence_operator_carrying_a_value_is_refused_rather_than_ignoring_it() {
        let error = CompiledQuery::compile(&query(vec![property_condition(
            "status",
            PropertyOperator::Exists,
            Some(json!("draft")),
        )]))
        .expect_err("refused");
        assert!(error.message.contains("exists"), "{}", error.message);
    }

    #[test]
    fn a_property_condition_on_tags_or_aliases_is_refused_rather_than_answering_missing() {
        for (name, hint) in [("tags", "tag condition"), ("aliases", "projected")] {
            let error = CompiledQuery::compile(&query(vec![property_condition(
                name,
                PropertyOperator::Eq,
                Some(json!("anything")),
            )]))
            .expect_err("refused");
            assert_eq!(error.code, "invalid_query");
            assert!(error.message.contains(hint), "{}", error.message);
        }
        // Projecting them is still fine; only querying them is refused.
        assert!(
            CompiledQuery::compile(&NoteQuery {
                properties: vec!["tags".to_string(), "aliases".to_string()],
                ..query(vec![tag("topic")])
            })
            .is_ok()
        );
    }

    #[test]
    fn an_empty_tag_or_path_prefix_is_refused() {
        for condition in [
            tag("  #  "),
            NoteQueryCondition::PathPrefix {
                prefix: " / ".to_string(),
            },
        ] {
            let error = CompiledQuery::compile(&query(vec![condition])).expect_err("refused");
            assert_eq!(error.code, "invalid_query");
        }
    }

    #[test]
    fn a_request_past_the_term_ceiling_is_refused() {
        let mut wide = query(
            (0..MAX_QUERY_TERMS + 1)
                .map(|n| tag(&n.to_string()))
                .collect(),
        );
        let error = CompiledQuery::compile(&wide).expect_err("refused");
        assert!(error.message.contains("conditions"), "{}", error.message);

        wide.conditions = vec![tag("topic")];
        wide.properties = (0..MAX_QUERY_TERMS + 1).map(|n| n.to_string()).collect();
        let error = CompiledQuery::compile(&wide).expect_err("refused");
        assert!(error.message.contains("properties"), "{}", error.message);
    }

    #[test]
    fn nested_tags_match_the_branch_and_never_a_namesake() {
        assert!(tag_matches("topic/sub", "topic"));
        assert!(tag_matches("topic", "topic"));
        assert!(!tag_matches("topical", "topic"));
    }

    #[test]
    fn a_path_prefix_selects_a_folder_and_not_a_namesake_folder() {
        assert!(path_under("notes/a.md", "notes"));
        assert!(path_under("notes/deep/a.md", "notes"));
        assert!(path_under("notes", "notes"));
        assert!(!path_under("notes-archive/a.md", "notes"));
        assert!(!path_under("other/a.md", "notes"));
    }

    #[test]
    fn a_path_prefix_ignores_case_and_surrounding_slashes() {
        let prefix = normalize_path_prefix("/Notes/Reference/").expect("prefix");
        assert_eq!(prefix, "notes/reference");
        assert!(path_under("Notes/Reference/A.md", &prefix));
    }

    #[test]
    fn only_missing_selects_an_absent_property() {
        for operator in [
            PropertyOperator::Exists,
            PropertyOperator::Empty,
            PropertyOperator::NotEmpty,
            PropertyOperator::Eq,
            PropertyOperator::Ne,
            PropertyOperator::Lt,
            PropertyOperator::Gte,
        ] {
            assert!(
                !property_matches(None, operator, Some(&json!("draft"))),
                "{operator:?} must not select a Note without the property"
            );
        }
        assert!(property_matches(None, PropertyOperator::Missing, None));
        assert!(!property_matches(
            Some(&json!("draft")),
            PropertyOperator::Missing,
            None
        ));
    }

    #[test]
    fn emptiness_separates_a_blank_value_from_an_absent_one() {
        for empty in [json!(null), json!(""), json!("   "), json!([]), json!({})] {
            assert!(property_matches(
                Some(&empty),
                PropertyOperator::Empty,
                None
            ));
            assert!(property_matches(
                Some(&empty),
                PropertyOperator::Exists,
                None
            ));
            assert!(!property_matches(
                Some(&empty),
                PropertyOperator::NotEmpty,
                None
            ));
        }
        assert!(property_matches(
            Some(&json!("draft")),
            PropertyOperator::NotEmpty,
            None
        ));
        assert!(property_matches(
            Some(&json!(false)),
            PropertyOperator::NotEmpty,
            None
        ));
    }

    #[test]
    fn equality_crosses_integer_and_float_spellings_of_one_number() {
        assert!(property_matches(
            Some(&json!(1)),
            PropertyOperator::Eq,
            Some(&json!(1.0))
        ));
        assert!(!property_matches(
            Some(&json!(1)),
            PropertyOperator::Eq,
            Some(&json!("1"))
        ));
    }

    #[test]
    fn equality_reaches_inside_a_multi_value_property() {
        let actual = json!(["reference", "archive"]);
        assert!(property_matches(
            Some(&actual),
            PropertyOperator::Eq,
            Some(&json!("reference"))
        ));
        assert!(!property_matches(
            Some(&actual),
            PropertyOperator::Eq,
            Some(&json!("inbox"))
        ));
        assert!(property_matches(
            Some(&actual),
            PropertyOperator::Ne,
            Some(&json!("inbox"))
        ));
        // A list compared against a list is compared whole, not element-wise.
        assert!(property_matches(
            Some(&actual),
            PropertyOperator::Eq,
            Some(&json!(["reference", "archive"]))
        ));
    }

    #[test]
    fn ordered_comparison_orders_iso_dates_and_refuses_mismatched_types() {
        let date = json!("2026-08-01");
        assert!(property_matches(
            Some(&date),
            PropertyOperator::Lt,
            Some(&json!("2026-09-06"))
        ));
        assert!(!property_matches(
            Some(&date),
            PropertyOperator::Gt,
            Some(&json!("2026-09-06"))
        ));
        assert!(property_matches(
            Some(&date),
            PropertyOperator::Lte,
            Some(&json!("2026-08-01"))
        ));
        // A string has no position relative to a number, and a list none
        // relative to a scalar: neither selects rather than guessing.
        assert!(!property_matches(
            Some(&date),
            PropertyOperator::Lt,
            Some(&json!(5))
        ));
        assert!(!property_matches(
            Some(&json!(["a"])),
            PropertyOperator::Lt,
            Some(&json!("b"))
        ));
    }

    #[test]
    fn numbers_and_booleans_order_the_way_a_reader_expects() {
        assert!(property_matches(
            Some(&json!(3)),
            PropertyOperator::Gte,
            Some(&json!(3))
        ));
        assert!(property_matches(
            Some(&json!(2.5)),
            PropertyOperator::Lt,
            Some(&json!(10))
        ));
        assert!(property_matches(
            Some(&json!(true)),
            PropertyOperator::Gt,
            Some(&json!(false))
        ));
    }

    #[test]
    fn the_limit_clamps_to_the_advertised_bounds_inside_the_core() {
        let compiled = |limit| {
            CompiledQuery::compile(&NoteQuery {
                limit,
                ..query(vec![tag("topic")])
            })
            .expect("compile")
            .limit
        };
        assert_eq!(compiled(None), 50);
        // Zero would otherwise answer every query with no rows and
        // `truncated: true`, which reads as a Vault problem.
        assert_eq!(compiled(Some(0)), 1);
        assert_eq!(compiled(Some(10_000)), 200);
        assert_eq!(compiled(Some(7)), 7);
    }

    #[test]
    fn a_condition_round_trips_through_its_wire_shape() {
        let condition =
            property_condition("review-date", PropertyOperator::Lt, Some(json!("2026")));
        let wire = serde_json::to_value(&condition).expect("serialize");
        assert_eq!(
            wire,
            json!({"type":"property","name":"review-date","operator":"lt","value":"2026"})
        );
        assert_eq!(
            serde_json::from_value::<NoteQueryCondition>(wire).expect("deserialize"),
            condition
        );

        let wire = serde_json::to_value(tag("topic/sub")).expect("serialize");
        assert_eq!(wire, json!({"type":"tag","tag":"topic/sub"}));
    }
}
