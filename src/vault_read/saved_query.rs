//! Saved queries: query definitions stored in a Note's fenced `base` blocks,
//! evaluated against that Note's own Vault every time they are read (#275).
//!
//! ADR-21 fixes the rules this module answers to. A result is derived state:
//! it is computed here, on demand, and never written into the Markdown, so the
//! indexer, search, backlinks, statistics and the graph never see it. It reads
//! only the Vault its Note lives in. And it implements a documented subset of
//! Obsidian's Bases syntax, refusing everything else by name rather than
//! applying part of a definition it did not understand.
//!
//! The supported subset:
//!
//! - `filters` at the top level and on the one view, each a string expression
//!   or an `and` / `or` / `not` list of them, nested to any sensible depth.
//! - Expressions comparing a note property (`price`, `note.price`,
//!   `note["next-payment"]`) or a file fact (`file.name`, `file.basename`,
//!   `file.path`, `file.folder`) against a string, number, boolean, `null`,
//!   `now()` or `today()`, with `==`, `!=`, `<`, `<=`, `>`, `>=`, `&&`, `||`,
//!   `!` and parentheses.
//! - `file.hasTag(...)`, `file.inFolder(...)` and `<property>.isEmpty()`.
//! - Exactly one view, carrying an optional `name`, `limit`, `order` (its
//!   columns, left to right) and `filters`.
//!
//! Refusal splits by effect (#276). A construct that could change which rows
//! appear refuses the whole saved query, naming the construct. One that only
//! changes how rows are drawn, `groupBy`, `summaries` or a view `type` other
//! than `table`, is set aside and reported in `ignored`, and the rows are
//! drawn in full. The outcome keeps refused, empty and populated apart.
//!
//! The conditions compile into the same [`CompiledCondition`] tree
//! `query_notes` evaluates, so a saved query and a caller-supplied query cannot
//! disagree about what a tag, a folder or a property comparison means.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_yaml_ng::Value as Yaml;

use crate::cache::parse::{frontmatter_span, parse_fence_marker};
use crate::cache::vault_snapshots::VaultSnapshotNote;
use crate::vault_registry::VaultId;

use super::query::{CompiledCondition, PropertyOperator, Subject};

/// The most saved queries one Note may hold. Each is evaluated against the
/// whole Vault, so without this a Note could multiply its way past the
/// per-query ceiling simply by repeating a block.
pub(super) const MAX_SAVED_QUERIES_PER_NOTE: usize = 10;

/// The largest definition read. Far past anything a person writes by hand.
const MAX_DEFINITION_BYTES: usize = 16 * 1024;

/// How deeply `and` / `or` / `not` lists and parentheses may nest before a
/// definition is refused, so a pathological one cannot exhaust the stack.
const MAX_NESTING: usize = 32;

/// The ceiling Hatchdoor holds every saved query to, whatever its definition
/// says. Separate from the definition's own `limit`, which the author sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SavedQueryCeiling {
    /// The most Notes one saved query may scan. A Vault holding more stops the
    /// query rather than evaluating part of it.
    pub(super) max_scanned_notes: usize,
    /// The most rows one saved query may return.
    pub(super) max_rows: usize,
}

impl SavedQueryCeiling {
    pub(super) const ENFORCED: Self = Self {
        max_scanned_notes: 20_000,
        max_rows: 500,
    };
}

/// The instant a saved query is evaluated at, as the two values its
/// definition can name. Frontmatter dates are ISO-8601 strings, which order
/// correctly byte by byte, so the current time is one too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EvaluationClock {
    now: String,
    today: String,
}

impl EvaluationClock {
    /// The server's local time at this moment. Taken once per read, so every
    /// saved query in one Note agrees on what "now" is.
    pub(super) fn current() -> Self {
        Self::at(chrono::Local::now().naive_local())
    }

    pub(super) fn at(instant: chrono::NaiveDateTime) -> Self {
        Self {
            now: instant.format("%Y-%m-%dT%H:%M:%S").to_string(),
            today: instant.format("%Y-%m-%d").to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Every saved query one Note holds, in the order they appear in it, plus what
/// is wrong with the Note's `hatchdoor-query` markers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedQueriesResponse {
    pub vault_id: VaultId,
    pub slug: String,
    pub queries: Vec<SavedQueryResult>,
    /// Marker problems belong to the Note rather than to one saved query: an
    /// orphaned marker names no saved query at all, and a repeated name
    /// belongs to every saved query claiming it. None of them changes a row.
    #[serde(default)]
    pub marker_problems: Vec<SavedQueryMarkerProblem>,
}

/// One saved query and what evaluating it produced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedQueryResult {
    /// The name from a `hatchdoor-query` marker, when the block has a usable
    /// one. Two saved queries in one Note may claim the same name; that is
    /// reported in `marker_problems`, and such a name addresses neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The definition exactly as it sits inside its fence, so a reader can
    /// tell which block of the Note this result belongs to.
    pub source: String,
    #[serde(flatten)]
    pub outcome: SavedQueryOutcome,
}

/// How a saved query ended. Refused, empty and populated are distinct
/// variants rather than a row list plus a message, so no caller can present a
/// broken definition as an empty answer by accident (ADR-21 part 3). A
/// `Populated` outcome always holds at least one row; zero rows is `Empty`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SavedQueryOutcome {
    /// Read and evaluated, and these Notes qualified.
    Populated(SavedQueryTable),
    /// Read and evaluated, and no Note qualified. A real answer.
    Empty(SavedQueryEmpty),
    /// The definition could not be read, or it uses something that would
    /// change which rows appear and that Hatchdoor does not support, so no row
    /// was computed at all.
    Refused(SavedQueryRefusal),
    /// The definition is fine but evaluating it would pass Hatchdoor's
    /// ceiling, so evaluation stopped rather than returning part of an answer.
    Stopped { message: String },
}

impl SavedQueryOutcome {
    /// The outcome for an evaluated saved query, which is `Empty` exactly
    /// when no row qualified. Evaluation builds either variant only here.
    fn evaluated(table: SavedQueryTable) -> Self {
        if table.rows.is_empty() {
            Self::Empty(SavedQueryEmpty {
                view_name: table.view_name,
                columns: table.columns,
                ignored: table.ignored,
            })
        } else {
            Self::Populated(table)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedQueryTable {
    /// The view's `name`, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_name: Option<String>,
    pub columns: Vec<SavedQueryColumn>,
    pub rows: Vec<SavedQueryRow>,
    /// Present when more Notes qualified than the table shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<SavedQueryTruncation>,
    /// Presentation instructions the definition gives and Hatchdoor does not
    /// carry out. The rows are complete and correct without them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignored: Vec<SavedQueryIgnored>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedQueryEmpty {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_name: Option<String>,
    pub columns: Vec<SavedQueryColumn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignored: Vec<SavedQueryIgnored>,
}

/// Why a saved query was not evaluated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedQueryRefusal {
    /// The piece of the definition Hatchdoor could not use, as the author
    /// wrote it where that is possible: `daysUntil()`, `formulas`, `YAML`, a
    /// whole filter expression.
    pub construct: String,
    /// A sentence for a person, naming the construct and what is wrong.
    pub message: String,
}

fn refuse(construct: impl Into<String>, message: impl Into<String>) -> SavedQueryRefusal {
    SavedQueryRefusal {
        construct: construct.into(),
        message: message.into(),
    }
}

/// A presentation instruction that was not carried out. Only an instruction
/// that cannot change which rows appear is ever ignored; anything else
/// refuses the saved query.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedQueryIgnored {
    /// The instruction as the definition gives it: `groupBy`, `summaries`,
    /// `type: cards`.
    pub instruction: String,
    pub message: String,
}

/// Something wrong with a `hatchdoor-query` marker. A marker only names a
/// saved query, so none of these changes a row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "problem", rename_all = "snake_case")]
pub enum SavedQueryMarkerProblem {
    /// A marker with no `base` block after it, so it names nothing.
    Orphaned {
        /// The name as the marker writes it.
        name: String,
        /// The marker's line in the Note file, counting from 1.
        line: usize,
        message: String,
    },
    /// A marker whose name is not a slug. The block after it is unnamed.
    UnusableName {
        name: String,
        /// The saved query's position in `queries`.
        query: usize,
        message: String,
    },
    /// Two or more saved queries claim one name, so the name addresses none
    /// of them until only one does. Their rows are unaffected.
    DuplicateName {
        name: String,
        /// The positions in `queries` of every saved query claiming it.
        queries: Vec<usize>,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedQueryColumn {
    /// The property as the definition's `order` names it, `file.name` or
    /// `note.price`.
    pub id: String,
    /// The name a reader sees: `price` for `note.price`, `name` for
    /// `file.name`.
    pub label: String,
}

/// One qualifying Note, Vault-qualified, with one cell per column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedQueryRow {
    pub vault_id: VaultId,
    pub title: String,
    pub slug: String,
    pub relative_path: String,
    /// Positionally matching `columns`. A property the Note does not carry is
    /// `null`.
    pub cells: Vec<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedQueryTruncation {
    pub reason: SavedQueryTruncationReason,
    /// How many rows the table holds.
    pub shown: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedQueryTruncationReason {
    /// The view's own `limit` held rows back, as its author asked.
    DefinitionLimit,
    /// Hatchdoor's row ceiling held rows back, whatever the definition asked.
    Ceiling,
}

// ---------------------------------------------------------------------------
// Finding the blocks
// ---------------------------------------------------------------------------

/// One `base` block as it sits in the Note, before anything is parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SavedQueryBlock {
    name: MarkerName,
    source: String,
}

/// What the `hatchdoor-query` marker before a block, if any, says about its
/// name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MarkerName {
    /// No marker precedes the block. A name is optional.
    Absent,
    Named(String),
    /// A marker is there, but what it names is not a slug. Holds the text as
    /// written, so the notice can quote it.
    Unusable(String),
}

impl MarkerName {
    /// The name as the marker writes it.
    fn written(&self) -> &str {
        match self {
            Self::Absent => "",
            Self::Named(name) | Self::Unusable(name) => name,
        }
    }
}

/// A `hatchdoor-query` marker with no `base` block after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OrphanedMarker {
    /// The name as the marker writes it, usable or not.
    name: String,
    /// The marker's line in the Note file, counting from 1.
    line: usize,
}

/// What a Note's Markdown holds for saved queries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NoteSavedQueries {
    blocks: Vec<SavedQueryBlock>,
    orphaned_markers: Vec<OrphanedMarker>,
}

/// Every fenced `base` block in `markdown`, in document order, each paired
/// with the `hatchdoor-query` marker preceding it, and every marker that
/// precedes no block. Only blank lines may sit between a marker and its
/// block; anything else detaches it.
///
/// The frontmatter is skipped: it is YAML, not Markdown, and a fence inside a
/// multi-line string there is not a block anyone rendered.
pub(super) fn saved_query_blocks(markdown: &str) -> NoteSavedQueries {
    let (body, first_line) = match frontmatter_span(markdown) {
        Some((_, end)) => {
            let head = markdown.get(..end + 4).unwrap_or(markdown);
            (
                markdown.get(end + 4..).unwrap_or(""),
                head.matches('\n').count() + 1,
            )
        }
        None => (markdown, 1),
    };

    let mut found = NoteSavedQueries {
        blocks: Vec::new(),
        orphaned_markers: Vec::new(),
    };
    let mut pending_marker: Option<(MarkerName, usize)> = None;
    let mut orphan = |pending: Option<(MarkerName, usize)>| {
        if let Some((name, line)) = pending {
            found.orphaned_markers.push(OrphanedMarker {
                name: name.written().to_string(),
                line,
            });
        }
    };
    let mut open: Option<OpenFence> = None;
    let mut blocks = Vec::new();

    for (offset, raw_line) in body.split('\n').enumerate() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let indent = leading_spaces(line);
        let trimmed = line.trim_start();

        if let Some(fence) = open.as_mut() {
            if fence.closes_on(trimmed) {
                let fence = open.take().expect("an open fence");
                if let Some(block) = fence.block {
                    blocks.push(block.finish());
                }
            } else if let Some(block) = fence.block.as_mut() {
                block.lines.push(strip_indent(line, fence.indent));
            }
            continue;
        }

        if indent <= 3
            && let Some((marker, len)) = parse_fence_marker(trimmed)
        {
            let info = trimmed[len..].trim();
            let is_base = info
                .split_whitespace()
                .next()
                .is_some_and(|language| language == "base");
            let name = if is_base {
                pending_marker
                    .take()
                    .map_or(MarkerName::Absent, |(name, _)| name)
            } else {
                orphan(pending_marker.take());
                MarkerName::Absent
            };
            open = Some(OpenFence {
                marker,
                len,
                indent,
                block: is_base.then(|| PendingBlock {
                    name,
                    lines: Vec::new(),
                }),
            });
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }
        orphan(pending_marker.take());
        pending_marker = query_marker(trimmed).map(|name| (name, first_line + offset));
    }
    orphan(pending_marker.take());

    // CommonMark runs an unclosed fence to the end of the document, and the
    // note page renders it that way, so it is still a block.
    if let Some(OpenFence {
        block: Some(block), ..
    }) = open
    {
        blocks.push(block.finish());
    }
    found.blocks = blocks;
    found
}

struct OpenFence {
    marker: u8,
    len: usize,
    indent: usize,
    /// `None` for a fence of any other language, which is skipped whole.
    block: Option<PendingBlock>,
}

impl OpenFence {
    fn closes_on(&self, trimmed: &str) -> bool {
        parse_fence_marker(trimmed).is_some_and(|(marker, len)| {
            marker == self.marker && len >= self.len && trimmed[len..].trim().is_empty()
        })
    }
}

struct PendingBlock {
    name: MarkerName,
    lines: Vec<String>,
}

impl PendingBlock {
    fn finish(self) -> SavedQueryBlock {
        SavedQueryBlock {
            name: self.name,
            source: self.lines.join("\n"),
        }
    }
}

fn leading_spaces(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// A content line with up to `indent` leading spaces removed, the way
/// CommonMark reads a fence that is itself indented.
fn strip_indent(line: &str, indent: usize) -> String {
    line[leading_spaces(line).min(indent)..].to_string()
}

/// The name a `<!-- hatchdoor-query: name -->` line gives the block after it,
/// or `None` for any line that is not a marker at all.
fn query_marker(trimmed: &str) -> Option<MarkerName> {
    let inner = trimmed.strip_prefix("<!--")?.strip_suffix("-->")?.trim();
    let name = inner.strip_prefix("hatchdoor-query:")?.trim();
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    Some(if valid {
        MarkerName::Named(name.to_string())
    } else {
        MarkerName::Unusable(name.to_string())
    })
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// Evaluate every saved query in one Note against its Vault's Notes.
///
/// The Vault's Notes are handed in once, so a Note holding several saved
/// queries reads its Vault once. The scan ceiling is one budget for the whole
/// Note rather than one per query, so repeating a block cannot multiply the
/// work a single read does past what one saved query may do.
///
/// A name never changes which rows appear, so a marker problem is reported
/// beside the results and every row is still computed (ADR-21 part 3).
pub(super) fn evaluate_saved_queries(
    found: NoteSavedQueries,
    vault_id: VaultId,
    notes: &[VaultSnapshotNote],
    clock: &EvaluationClock,
    ceiling: SavedQueryCeiling,
) -> EvaluatedSavedQueries {
    let mut marker_problems: Vec<SavedQueryMarkerProblem> = found
        .orphaned_markers
        .into_iter()
        .map(|marker| SavedQueryMarkerProblem::Orphaned {
            message: format!(
                "The marker naming \"{}\" is not followed by a base block, so it names nothing.",
                marker.name
            ),
            name: marker.name,
            line: marker.line,
        })
        .collect();
    let mut claims: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut scan_budget = ceiling.max_scanned_notes;

    let queries = found
        .blocks
        .into_iter()
        .enumerate()
        .map(|(position, block)| {
            let name = match block.name {
                MarkerName::Absent => None,
                MarkerName::Named(name) => {
                    claims.entry(name.clone()).or_default().push(position);
                    Some(name)
                }
                MarkerName::Unusable(name) => {
                    marker_problems.push(SavedQueryMarkerProblem::UnusableName {
                        message: format!(
                            "\"{name}\" is not a usable name, so this saved query has no name. A name is lowercase letters, digits and hyphens."
                        ),
                        name,
                        query: position,
                    });
                    None
                }
            };
            let outcome = if position >= MAX_SAVED_QUERIES_PER_NOTE {
                SavedQueryOutcome::Stopped {
                    message: format!(
                        "A note may hold at most {MAX_SAVED_QUERIES_PER_NOTE} saved queries, and this is number {}.",
                        position + 1
                    ),
                }
            } else {
                match parse_definition(&block.source, clock) {
                    Ok(definition) => {
                        evaluate(definition, vault_id, notes, ceiling, &mut scan_budget)
                    }
                    Err(refusal) => SavedQueryOutcome::Refused(refusal),
                }
            };
            SavedQueryResult {
                name,
                source: block.source,
                outcome,
            }
        })
        .collect();

    marker_problems.extend(
        claims
            .into_iter()
            .filter(|(_, queries)| queries.len() > 1)
            .map(|(name, queries)| SavedQueryMarkerProblem::DuplicateName {
                message: format!(
                    "{} saved queries in this note are named \"{name}\", so none of them can be addressed by that name until only one is.",
                    queries.len()
                ),
                name,
                queries,
            }),
    );

    EvaluatedSavedQueries {
        queries,
        marker_problems,
    }
}

/// Every saved query in one Note, evaluated, with its marker problems.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct EvaluatedSavedQueries {
    pub(super) queries: Vec<SavedQueryResult>,
    pub(super) marker_problems: Vec<SavedQueryMarkerProblem>,
}

fn evaluate(
    definition: Definition,
    vault_id: VaultId,
    notes: &[VaultSnapshotNote],
    ceiling: SavedQueryCeiling,
    scan_budget: &mut usize,
) -> SavedQueryOutcome {
    if notes.len() > ceiling.max_scanned_notes {
        return SavedQueryOutcome::Stopped {
            message: format!(
                "This saved query would scan {} notes, and Hatchdoor stops a saved query at {}.",
                notes.len(),
                ceiling.max_scanned_notes
            ),
        };
    }
    if notes.len() > *scan_budget {
        return SavedQueryOutcome::Stopped {
            message: format!(
                "The saved queries before this one in the note have already scanned {} notes, and Hatchdoor stops one note's saved queries at {} together.",
                ceiling.max_scanned_notes - *scan_budget,
                ceiling.max_scanned_notes
            ),
        };
    }
    *scan_budget -= notes.len();

    let mut selected: Vec<&VaultSnapshotNote> = notes
        .iter()
        .filter(|note| definition.condition.matches(note))
        .collect();
    selected.sort_by(|left, right| row_order(left, right));

    let (shown, reason) = match definition.limit {
        Some(limit) if limit <= ceiling.max_rows => {
            (limit, SavedQueryTruncationReason::DefinitionLimit)
        }
        _ => (ceiling.max_rows, SavedQueryTruncationReason::Ceiling),
    };
    let truncated = (selected.len() > shown).then_some(SavedQueryTruncation { reason, shown });
    selected.truncate(shown);

    SavedQueryOutcome::evaluated(SavedQueryTable {
        rows: selected
            .into_iter()
            .map(|note| SavedQueryRow {
                vault_id,
                title: note.title.clone(),
                slug: note.slug.clone(),
                relative_path: note.relative_path.clone(),
                cells: definition
                    .columns
                    .iter()
                    .map(|column| column.cell(note))
                    .collect(),
            })
            .collect(),
        view_name: definition.view_name,
        columns: definition
            .columns
            .into_iter()
            .map(|column| column.wire)
            .collect(),
        truncated,
        ignored: definition.ignored,
    })
}

/// The one order rows come back in: by title, ignoring case, then by path and
/// slug so two Notes sharing a title still land in the same order on every
/// evaluation. Row sorting is not part of the Bases format, so the definition
/// cannot change this (ADR-21).
fn row_order(left: &VaultSnapshotNote, right: &VaultSnapshotNote) -> Ordering {
    left.title
        .to_lowercase()
        .cmp(&right.title.to_lowercase())
        .then_with(|| left.title.cmp(&right.title))
        .then_with(|| left.relative_path.cmp(&right.relative_path))
        .then_with(|| left.slug.cmp(&right.slug))
}

// ---------------------------------------------------------------------------
// The definition
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Definition {
    condition: CompiledCondition,
    columns: Vec<Column>,
    view_name: Option<String>,
    limit: Option<usize>,
    ignored: Vec<SavedQueryIgnored>,
}

#[derive(Debug)]
struct Column {
    /// The column as a reader is told about it.
    wire: SavedQueryColumn,
    value: ColumnValue,
}

#[derive(Debug)]
enum ColumnValue {
    Subject(Subject),
    /// `file.tags`: the Note's parsed tags, which the frontmatter parser lifts
    /// out of the property map.
    Tags,
}

impl Column {
    fn cell(&self, note: &VaultSnapshotNote) -> Value {
        match &self.value {
            ColumnValue::Subject(subject) => subject
                .value(note)
                .map(|value| value.into_owned())
                .unwrap_or(Value::Null),
            ColumnValue::Tags => Value::from(note.metadata.tags.clone()),
        }
    }
}

/// Instructions that change only how rows are drawn, never which rows appear.
/// A definition carrying one is still evaluated, and the instruction is
/// reported as ignored rather than refusing the whole saved query.
fn ignore_grouping() -> SavedQueryIgnored {
    SavedQueryIgnored {
        instruction: "groupBy".to_string(),
        message: "Grouping is not supported, so the rows are shown ungrouped.".to_string(),
    }
}

fn ignore_summaries() -> SavedQueryIgnored {
    SavedQueryIgnored {
        instruction: "summaries".to_string(),
        message: "Summaries are not supported, so no summary row is shown.".to_string(),
    }
}

fn ignore_view_type(view_type: &str) -> SavedQueryIgnored {
    SavedQueryIgnored {
        instruction: format!("type: {view_type}"),
        message: format!(
            "Hatchdoor draws table views only, so this {view_type} view is shown as a table."
        ),
    }
}

/// Parse and compile one definition, or say what in it Hatchdoor does not
/// support. Nothing that affects which rows appear is partially applied: the
/// first such construct refuses the whole saved query. A construct affecting
/// only presentation is set aside and named in `ignored`.
fn parse_definition(
    source: &str,
    clock: &EvaluationClock,
) -> Result<Definition, SavedQueryRefusal> {
    if source.len() > MAX_DEFINITION_BYTES {
        return Err(refuse(
            "definition size",
            format!(
                "This saved query is longer than the {MAX_DEFINITION_BYTES} bytes Hatchdoor reads."
            ),
        ));
    }
    if source.trim().is_empty() {
        return Err(refuse("empty definition", "This saved query is empty."));
    }
    let yaml: Yaml = serde_yaml_ng::from_str(source).map_err(|error| {
        refuse(
            "YAML",
            format!("This saved query is not valid YAML: {error}"),
        )
    })?;
    let Yaml::Mapping(top) = yaml else {
        return Err(refuse(
            "definition",
            "A saved query must be a set of keys such as filters and views.",
        ));
    };

    let mut conditions = Vec::new();
    let mut view = None;
    let mut ignored = Vec::new();
    for (key, value) in &top {
        match yaml_key(key)? {
            "filters" => conditions.push(filter(value, clock, 0)?),
            "views" => view = single_view(value)?,
            "summaries" => ignored.push(ignore_summaries()),
            other => return Err(unsupported_key(other, "at the top of a saved query")),
        }
    }

    let (view_name, limit, columns) = match view {
        Some(view) => {
            if let Some(filters) = view.filters {
                conditions.push(filter(filters, clock, 0)?);
            }
            ignored.extend(view.ignored);
            (view.name, view.limit, view.columns)
        }
        None => (None, None, Vec::new()),
    };
    let columns = if columns.is_empty() {
        vec![column("file.name")?]
    } else {
        columns
    };
    // `summaries` may sit at the top and on the view; say it once.
    let mut seen = BTreeSet::new();
    ignored.retain(|instruction| seen.insert(instruction.instruction.clone()));

    Ok(Definition {
        condition: CompiledCondition::All(conditions),
        columns,
        view_name,
        limit,
        ignored,
    })
}

struct View<'a> {
    name: Option<String>,
    limit: Option<usize>,
    columns: Vec<Column>,
    filters: Option<&'a Yaml>,
    ignored: Vec<SavedQueryIgnored>,
}

/// The one view a definition carries, or `None` for an empty list, which
/// draws the same default table as a definition with no `views` at all.
fn single_view(value: &Yaml) -> Result<Option<View<'_>>, SavedQueryRefusal> {
    let Yaml::Sequence(views) = value else {
        return Err(refuse("views", "views must be a list."));
    };
    let view = match views.as_slice() {
        [] => return Ok(None),
        [view] => view,
        // Which view a reader meant cannot be known, and two views may filter
        // differently, so this is not a presentation choice to set aside.
        _ => {
            return Err(refuse(
                "views",
                format!(
                    "This saved query defines {} views, and Hatchdoor supports exactly one.",
                    views.len()
                ),
            ));
        }
    };
    let Yaml::Mapping(view) = view else {
        return Err(refuse(
            "views",
            "A view must be a set of keys such as type and order.",
        ));
    };

    let mut parsed = View {
        name: None,
        limit: None,
        columns: Vec::new(),
        filters: None,
        ignored: Vec::new(),
    };
    for (key, value) in view {
        match yaml_key(key)? {
            "type" => match value.as_str() {
                Some("table") => {}
                Some(other) => parsed.ignored.push(ignore_view_type(other)),
                None => return Err(refuse("type", "A view's type must be text.")),
            },
            "name" => {
                parsed.name = Some(
                    value
                        .as_str()
                        .ok_or_else(|| refuse("name", "A view's name must be text."))?
                        .to_string(),
                );
            }
            "limit" => {
                parsed.limit = Some(
                    value
                        .as_u64()
                        .filter(|limit| *limit > 0)
                        .and_then(|limit| usize::try_from(limit).ok())
                        .ok_or_else(|| {
                            refuse("limit", "A view's limit must be a whole number above zero.")
                        })?,
                );
            }
            "order" => {
                let Yaml::Sequence(ids) = value else {
                    return Err(refuse(
                        "order",
                        "A view's order must be a list of properties.",
                    ));
                };
                parsed.columns =
                    ids.iter()
                        .map(|id| {
                            column(id.as_str().ok_or_else(|| {
                                refuse("order", "Each column in order must be text.")
                            })?)
                        })
                        .collect::<Result<_, _>>()?;
            }
            "filters" => parsed.filters = Some(value),
            "groupBy" => parsed.ignored.push(ignore_grouping()),
            "summaries" => parsed.ignored.push(ignore_summaries()),
            other => return Err(unsupported_key(other, "in a view")),
        }
    }
    Ok(Some(parsed))
}

fn yaml_key(key: &Yaml) -> Result<&str, SavedQueryRefusal> {
    key.as_str()
        .ok_or_else(|| refuse("key", "Every key in a saved query must be text."))
}

fn unsupported_key(key: &str, place: &str) -> SavedQueryRefusal {
    refuse(
        key,
        format!("\"{key}\" {place} is not supported by Hatchdoor."),
    )
}

/// One column the view's `order` names.
fn column(id: &str) -> Result<Column, SavedQueryRefusal> {
    let id = id.trim();
    let (label, value) = if id == "file.tags" {
        ("tags".to_string(), ColumnValue::Tags)
    } else if let Some(member) = id.strip_prefix("file.")
        && let Some(subject) = Subject::file_member(member)
    {
        (member.to_string(), ColumnValue::Subject(subject))
    } else {
        match property_reference(id) {
            Some(name) => (name.clone(), ColumnValue::Subject(Subject::Property(name))),
            None => {
                return Err(refuse(id, format!("The column \"{id}\" is not supported.")));
            }
        }
    };
    Ok(Column {
        wire: SavedQueryColumn {
            id: id.to_string(),
            label,
        },
        value,
    })
}

/// The frontmatter property a column id names, or `None` for an id that is
/// not a plain property. `formula.`, `this.` and unknown `file.` members are
/// never read as a property of that name.
fn property_reference(id: &str) -> Option<String> {
    if let Some(rest) = id.strip_prefix("note.") {
        return Some(rest.to_string()).filter(|name| is_identifier(name));
    }
    if let Some(inner) = id
        .strip_prefix("note[")
        .and_then(|rest| rest.strip_suffix(']'))
    {
        let inner = inner.trim();
        let unquoted = inner
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .or_else(|| {
                inner
                    .strip_prefix('\'')
                    .and_then(|rest| rest.strip_suffix('\''))
            });
        return unquoted
            .map(ToOwned::to_owned)
            .filter(|name| !name.is_empty());
    }
    if ["formula.", "this.", "file."]
        .iter()
        .any(|namespace| id.starts_with(namespace))
    {
        return None;
    }
    is_identifier(id).then(|| id.to_string())
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '-')
}

/// One entry of a `filters` tree: an expression string, or an `and` / `or` /
/// `not` list of further entries.
fn filter(
    node: &Yaml,
    clock: &EvaluationClock,
    depth: usize,
) -> Result<CompiledCondition, SavedQueryRefusal> {
    if depth > MAX_NESTING {
        return Err(refuse(
            "filters",
            format!("The filters nest more than {MAX_NESTING} levels deep."),
        ));
    }
    match node {
        Yaml::String(expression) => Expression::parse(expression, clock),
        Yaml::Mapping(mapping) => {
            let mut entries = mapping.iter();
            let (Some((key, value)), None) = (entries.next(), entries.next()) else {
                return Err(refuse(
                    "filters",
                    "Each filter group must have exactly one of and, or, not.",
                ));
            };
            let combinator = yaml_key(key)?;
            let Yaml::Sequence(items) = value else {
                return Err(refuse(
                    combinator,
                    format!("The {combinator} filter group must hold a list."),
                ));
            };
            if items.is_empty() {
                return Err(refuse(
                    combinator,
                    format!("The {combinator} filter group is empty."),
                ));
            }
            let inner = items
                .iter()
                .map(|item| filter(item, clock, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            match combinator {
                "and" => Ok(CompiledCondition::All(inner)),
                "or" => Ok(CompiledCondition::Any(inner)),
                // Bases reads `not` as "none of these holds".
                "not" => Ok(CompiledCondition::Not(Box::new(CompiledCondition::Any(
                    inner,
                )))),
                other => Err(refuse(
                    other,
                    format!("The filter group \"{other}\" is not supported; use and, or, not."),
                )),
            }
        }
        _ => Err(refuse(
            "filters",
            "Each filter must be an expression in text, or an and, or, not group.",
        )),
    }
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Identifier(String),
    Text(String),
    Number(serde_json::Number),
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Comma,
    Dot,
    Compare(Comparison),
    And,
    Or,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Comparison {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
}

impl Comparison {
    /// The same comparison read from the other side: `5 < price` is
    /// `price > 5`.
    fn flipped(self) -> Self {
        match self {
            Self::Eq => Self::Eq,
            Self::Ne => Self::Ne,
            Self::Lt => Self::Gt,
            Self::Lte => Self::Gte,
            Self::Gt => Self::Lt,
            Self::Gte => Self::Lte,
        }
    }

    fn spelling(self) -> &'static str {
        match self {
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Lte => "<=",
            Self::Gt => ">",
            Self::Gte => ">=",
        }
    }
}

fn tokenize(expression: &str) -> Result<Vec<Token>, SavedQueryRefusal> {
    let chars: Vec<char> = expression.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        let next = chars.get(at + 1).copied();
        let (token, width) = match ch {
            _ if ch.is_whitespace() => {
                at += 1;
                continue;
            }
            '(' => (Token::LeftParen, 1),
            ')' => (Token::RightParen, 1),
            '[' => (Token::LeftBracket, 1),
            ']' => (Token::RightBracket, 1),
            ',' => (Token::Comma, 1),
            '.' if !next.is_some_and(|next| next.is_ascii_digit()) => (Token::Dot, 1),
            '&' if next == Some('&') => (Token::And, 2),
            '|' if next == Some('|') => (Token::Or, 2),
            '=' if next == Some('=') => (Token::Compare(Comparison::Eq), 2),
            '!' if next == Some('=') => (Token::Compare(Comparison::Ne), 2),
            '!' => (Token::Not, 1),
            '<' if next == Some('=') => (Token::Compare(Comparison::Lte), 2),
            '<' => (Token::Compare(Comparison::Lt), 1),
            '>' if next == Some('=') => (Token::Compare(Comparison::Gte), 2),
            '>' => (Token::Compare(Comparison::Gt), 1),
            '"' | '\'' => {
                let mut text = String::new();
                let mut end = at + 1;
                loop {
                    match chars.get(end) {
                        None => {
                            return Err(refuse(
                                expression,
                                format!("The expression \"{expression}\" has an unclosed quote."),
                            ));
                        }
                        Some(&close) if close == ch => break,
                        Some('\\') => {
                            let escaped = chars.get(end + 1).ok_or_else(|| {
                                refuse(
                                    expression,
                                    format!(
                                        "The expression \"{expression}\" has an unclosed quote."
                                    ),
                                )
                            })?;
                            text.push(*escaped);
                            end += 2;
                        }
                        Some(&other) => {
                            text.push(other);
                            end += 1;
                        }
                    }
                }
                (Token::Text(text), end + 1 - at)
            }
            _ if ch.is_ascii_digit()
                || (ch == '-' || ch == '.') && next.is_some_and(|next| next.is_ascii_digit()) =>
            {
                let mut end = at + 1;
                while chars
                    .get(end)
                    .is_some_and(|next| next.is_ascii_digit() || *next == '.')
                {
                    end += 1;
                }
                let literal: String = chars[at..end].iter().collect();
                let number = literal
                    .parse::<i64>()
                    .ok()
                    .map(serde_json::Number::from)
                    .or_else(|| {
                        literal
                            .parse::<f64>()
                            .ok()
                            .and_then(serde_json::Number::from_f64)
                    })
                    .ok_or_else(|| {
                        refuse(literal.clone(), format!("\"{literal}\" is not a number."))
                    })?;
                (Token::Number(number), end - at)
            }
            _ if ch.is_alphabetic() || ch == '_' => {
                let mut end = at + 1;
                while chars
                    .get(end)
                    .is_some_and(|next| next.is_alphanumeric() || *next == '_')
                {
                    end += 1;
                }
                (Token::Identifier(chars[at..end].iter().collect()), end - at)
            }
            other => {
                return Err(refuse(
                    other.to_string(),
                    format!(
                        "The expression \"{expression}\" uses \"{other}\", which is not supported."
                    ),
                ));
            }
        };
        tokens.push(token);
        at += width;
    }
    Ok(tokens)
}

/// What one side of a comparison, or a whole filter, turned out to be.
enum Term {
    /// Something read from each Note.
    Subject(Subject),
    /// A fixed value: a literal, `now()` or `today()`.
    Value(Value),
    /// Already a test in its own right: `file.hasTag(...)`, `x.isEmpty()`, or
    /// a parenthesised expression.
    Condition(CompiledCondition),
}

/// A recursive-descent reader for one filter expression.
struct Expression<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    at: usize,
    clock: &'a EvaluationClock,
    depth: usize,
}

impl<'a> Expression<'a> {
    fn parse(
        source: &'a str,
        clock: &'a EvaluationClock,
    ) -> Result<CompiledCondition, SavedQueryRefusal> {
        let mut expression = Self {
            source,
            tokens: tokenize(source)?,
            at: 0,
            clock,
            depth: 0,
        };
        if expression.tokens.is_empty() {
            return Err(refuse("filters", "A filter expression is empty."));
        }
        let condition = expression.or()?;
        if expression.at != expression.tokens.len() {
            return Err(expression.unexpected());
        }
        Ok(condition)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &Token) -> Result<(), SavedQueryRefusal> {
        if self.eat(token) {
            Ok(())
        } else {
            Err(self.unexpected())
        }
    }

    fn unexpected(&self) -> SavedQueryRefusal {
        self.refuse(format!(
            "The expression \"{}\" is not in the syntax Hatchdoor supports.",
            self.source
        ))
    }

    /// A refusal naming this whole expression, for a problem with its shape
    /// rather than with one name inside it.
    fn refuse(&self, message: String) -> SavedQueryRefusal {
        refuse(self.source, message)
    }

    fn nested<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, SavedQueryRefusal>,
    ) -> Result<T, SavedQueryRefusal> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            return Err(self.refuse(format!(
                "The expression \"{}\" nests more than {MAX_NESTING} levels deep.",
                self.source
            )));
        }
        let result = read(self);
        self.depth -= 1;
        result
    }

    fn or(&mut self) -> Result<CompiledCondition, SavedQueryRefusal> {
        let mut any = vec![self.and()?];
        while self.eat(&Token::Or) {
            any.push(self.and()?);
        }
        Ok(if any.len() == 1 {
            any.pop().expect("one condition")
        } else {
            CompiledCondition::Any(any)
        })
    }

    fn and(&mut self) -> Result<CompiledCondition, SavedQueryRefusal> {
        let mut all = vec![self.unary()?];
        while self.eat(&Token::And) {
            all.push(self.unary()?);
        }
        Ok(if all.len() == 1 {
            all.pop().expect("one condition")
        } else {
            CompiledCondition::All(all)
        })
    }

    fn unary(&mut self) -> Result<CompiledCondition, SavedQueryRefusal> {
        if self.eat(&Token::Not) {
            return self
                .nested(|expression| Ok(CompiledCondition::Not(Box::new(expression.unary()?))));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Result<CompiledCondition, SavedQueryRefusal> {
        let left = self.term()?;
        let Some(Token::Compare(comparison)) = self.peek().cloned() else {
            return match left {
                Term::Condition(condition) => Ok(condition),
                Term::Subject(_) | Term::Value(_) => Err(self.refuse(format!(
                    "The expression \"{}\" needs a comparison, such as == or >.",
                    self.source
                ))),
            };
        };
        self.at += 1;
        let right = self.term()?;
        match (left, right) {
            (Term::Subject(subject), Term::Value(value)) => {
                compare(subject, comparison, value, self.source)
            }
            (Term::Value(value), Term::Subject(subject)) => {
                compare(subject, comparison.flipped(), value, self.source)
            }
            _ => Err(self.refuse(format!(
                "The expression \"{}\" must compare a property with a value.",
                self.source
            ))),
        }
    }

    fn term(&mut self) -> Result<Term, SavedQueryRefusal> {
        match self.peek().cloned() {
            Some(Token::LeftParen) => {
                self.at += 1;
                let condition = self.nested(Self::or)?;
                self.expect(&Token::RightParen)?;
                Ok(Term::Condition(condition))
            }
            Some(Token::Text(text)) => {
                self.at += 1;
                Ok(Term::Value(Value::String(text)))
            }
            Some(Token::Number(number)) => {
                self.at += 1;
                Ok(Term::Value(Value::Number(number)))
            }
            Some(Token::Identifier(identifier)) => {
                self.at += 1;
                self.reference(identifier)
            }
            _ => Err(self.unexpected()),
        }
    }

    /// Everything that starts with a name: a literal keyword, a function, a
    /// file fact or file function, or a property, each optionally followed by
    /// `.isEmpty()`.
    fn reference(&mut self, identifier: String) -> Result<Term, SavedQueryRefusal> {
        let subject = match identifier.as_str() {
            "true" => return Ok(Term::Value(Value::Bool(true))),
            "false" => return Ok(Term::Value(Value::Bool(false))),
            "null" => return Ok(Term::Value(Value::Null)),
            "now" | "today" if self.peek() == Some(&Token::LeftParen) => {
                self.at += 1;
                self.expect(&Token::RightParen)?;
                let instant = if identifier == "now" {
                    &self.clock.now
                } else {
                    &self.clock.today
                };
                return Ok(Term::Value(Value::String(instant.clone())));
            }
            "file" => {
                self.expect(&Token::Dot)?;
                let Some(Token::Identifier(member)) = self.peek().cloned() else {
                    return Err(self.unexpected());
                };
                self.at += 1;
                match member.as_str() {
                    "hasTag" => return self.has_tag().map(Term::Condition),
                    "inFolder" => return self.in_folder().map(Term::Condition),
                    other => Subject::file_member(other).ok_or_else(|| {
                        refuse(
                            format!("file.{other}"),
                            format!("file.{other} is not supported in a saved query."),
                        )
                    })?,
                }
            }
            "note" if self.peek() == Some(&Token::Dot) => {
                self.at += 1;
                let Some(Token::Identifier(name)) = self.peek().cloned() else {
                    return Err(self.unexpected());
                };
                self.at += 1;
                self.property(name)?
            }
            "note" if self.peek() == Some(&Token::LeftBracket) => {
                self.at += 1;
                let Some(Token::Text(name)) = self.peek().cloned() else {
                    return Err(self.unexpected());
                };
                self.at += 1;
                self.expect(&Token::RightBracket)?;
                self.property(name)?
            }
            "formula" | "this" => {
                return Err(refuse(
                    identifier.clone(),
                    format!("{identifier} is not supported in a saved query."),
                ));
            }
            _ if self.peek() == Some(&Token::LeftParen) => {
                return Err(refuse(
                    format!("{identifier}()"),
                    format!("The function {identifier}() is not supported in a saved query."),
                ));
            }
            _ => self.property(identifier)?,
        };

        if self.eat(&Token::Dot) {
            let Some(Token::Identifier(method)) = self.peek().cloned() else {
                return Err(self.unexpected());
            };
            self.at += 1;
            // Named before its arguments are read, so the refusal points at
            // the method rather than at whatever it was given.
            if method != "isEmpty" {
                return Err(refuse(
                    format!("{method}()"),
                    format!("The method {method}() is not supported in a saved query."),
                ));
            }
            self.expect(&Token::LeftParen)?;
            self.expect(&Token::RightParen)?;
            return Ok(Term::Condition(is_empty(subject)));
        }
        Ok(Term::Subject(subject))
    }

    fn property(&self, name: String) -> Result<Subject, SavedQueryRefusal> {
        match name.as_str() {
            "tags" => Err(refuse(
                "tags",
                "Select by tag with file.hasTag(...) rather than the tags property.",
            )),
            "aliases" => Err(refuse(
                "aliases",
                "aliases cannot be compared in a saved query.",
            )),
            _ => Ok(Subject::Property(name)),
        }
    }

    /// The text arguments of a function call, after its name.
    fn text_arguments(&mut self, function: &str) -> Result<Vec<String>, SavedQueryRefusal> {
        self.expect(&Token::LeftParen)?;
        let mut arguments = Vec::new();
        loop {
            let Some(Token::Text(argument)) = self.peek().cloned() else {
                return Err(refuse(
                    function,
                    format!("{function}(...) takes one or more quoted names."),
                ));
            };
            self.at += 1;
            arguments.push(argument);
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        self.expect(&Token::RightParen)?;
        Ok(arguments)
    }

    fn has_tag(&mut self) -> Result<CompiledCondition, SavedQueryRefusal> {
        let tags = self
            .text_arguments("file.hasTag")?
            .iter()
            .map(|tag| {
                CompiledCondition::tag(tag).ok_or_else(|| {
                    refuse("file.hasTag", "file.hasTag(...) was given an empty tag.")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CompiledCondition::Any(tags))
    }

    fn in_folder(&mut self) -> Result<CompiledCondition, SavedQueryRefusal> {
        let [folder] = self
            .text_arguments("file.inFolder")?
            .try_into()
            .map_err(|_| {
                refuse(
                    "file.inFolder",
                    "file.inFolder(...) takes exactly one folder.",
                )
            })?;
        CompiledCondition::folder(&folder).ok_or_else(|| {
            refuse(
                "file.inFolder",
                "file.inFolder(...) was given an empty folder.",
            )
        })
    }
}

/// `subject.isEmpty()`: true for a property that is absent or holds nothing a
/// reader would see.
fn is_empty(subject: Subject) -> CompiledCondition {
    missing_or(subject, PropertyOperator::Empty, None)
}

/// The subject is absent, or present and passing `operator`. Bases reads a
/// missing property as `null`, so a test that holds for `null` must hold for
/// an absent property too.
fn missing_or(
    subject: Subject,
    operator: PropertyOperator,
    value: Option<Value>,
) -> CompiledCondition {
    CompiledCondition::Any(vec![
        CompiledCondition::Compare {
            subject: subject.clone(),
            operator: PropertyOperator::Missing,
            value: None,
        },
        CompiledCondition::Compare {
            subject,
            operator,
            value,
        },
    ])
}

/// One comparison between a subject and a fixed value, with Bases' reading of
/// a missing property: it is `null`, so `== null` selects it, `!=` anything
/// else selects it, and an ordered comparison against it never holds.
fn compare(
    subject: Subject,
    comparison: Comparison,
    value: Value,
    source: &str,
) -> Result<CompiledCondition, SavedQueryRefusal> {
    let equal = |subject: Subject, value: Value| {
        if value.is_null() {
            missing_or(subject, PropertyOperator::Eq, Some(Value::Null))
        } else {
            CompiledCondition::Compare {
                subject,
                operator: PropertyOperator::Eq,
                value: Some(value),
            }
        }
    };
    let ordered = |operator| {
        if value.is_null() {
            return Err(refuse(
                source,
                format!(
                    "The expression \"{source}\" orders against null with {}, which has no answer.",
                    comparison.spelling()
                ),
            ));
        }
        Ok(CompiledCondition::Compare {
            subject: subject.clone(),
            operator,
            value: Some(value.clone()),
        })
    };
    match comparison {
        Comparison::Eq => Ok(equal(subject, value)),
        Comparison::Ne => Ok(CompiledCondition::Not(Box::new(equal(subject, value)))),
        Comparison::Lt => ordered(PropertyOperator::Lt),
        Comparison::Lte => ordered(PropertyOperator::Lte),
        Comparison::Gt => ordered(PropertyOperator::Gt),
        Comparison::Gte => ordered(PropertyOperator::Gte),
    }
}

#[cfg(test)]
mod tests;
