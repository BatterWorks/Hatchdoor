//! Vault-wide tag rename (#242) and tag delete (#258).
//!
//! One call plans, a second call applies. The plan is every edit the rename
//! would make, resolved against the Vault as it stands, and its fingerprint is
//! a hash of those edits rather than anything the server remembers: the
//! applying call plans again and writes only if the two fingerprints match.
//! That is `expected_content_hash` scaled from one note to a set of them.
//!
//! A tag is what the indexer says it is. Inline tags are found by
//! [`inline_tags`], the same recognition the index stores tags with, so code
//! blocks and code spans are skipped by construction. The frontmatter side goes
//! through the shared in-place editor (`frontmatter.rs`), so a one-line list
//! stays a one-line list. Every rewritten note is then read back with
//! [`extract_tags`] and must carry exactly the tags the rename promised; a note
//! that does not refuses the whole plan. Nothing is written unless every note
//! can be edited, and a failure partway through the writes restores the notes
//! already written.
//!
//! A delete is the narrow sibling. It removes one exact tag from frontmatter
//! `tags` values and nothing else, so it refuses while any note carries the tag
//! inline or carries a tag nested beneath it: either would leave a tag search
//! for the deleted tag still finding notes. It shares the rename's handshake,
//! editor, backstop and journal.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::cache::parse::{
    InlineTag, content_hash, extract_tags, frontmatter_span, inline_tags, is_tag_char,
};
use crate::search::tag_matches;
use crate::vault::types::{NoteEntry, VaultIndex};

use super::frontmatter::{FrontmatterEdit, edit_frontmatter_block};
use super::fs_ops::MutationJournal;
use super::rewrites::read_note_text;
use super::types::{TextRewrite, WriteError};

/// A rename that was planned, or planned and applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRename {
    /// The tag being renamed, normalised: no `#`, lowercase.
    pub old_tag: String,
    pub new_tag: String,
    /// Whether this call wrote the notes below. A plan never does.
    pub applied: bool,
    /// Every note whose text the rename changes, in path order.
    pub notes: Vec<TagRenameNote>,
    /// Notes whose frontmatter `tags` change.
    pub frontmatter_notes: usize,
    /// Notes whose body carries an inline tag that changes.
    pub body_notes: usize,
    /// Notes that already carry `new_tag`, or a tag beneath it, before the
    /// rename. Anything above zero makes this rename a merge.
    pub already_tagged_notes: usize,
    /// The plan's fingerprint. `None` when there is nothing to change, since
    /// there is then nothing to confirm.
    pub plan_hash: Option<String>,
    /// Absolute paths this call wrote. Empty for a plan.
    pub affected_paths: Vec<PathBuf>,
}

/// One note the rename changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRenameNote {
    pub slug: String,
    pub relative_path: String,
    /// Its frontmatter `tags` change.
    pub frontmatter: bool,
    /// Its body carries an inline tag that changes.
    pub body: bool,
    /// The note's content hash once this call returns: its current hash for a
    /// plan, the rewritten note's hash once applied.
    pub content_hash: String,
}

/// A note whose tags cannot be renamed without touching text the rename was
/// not asked to change, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedTagNote {
    pub relative_path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagRenameError {
    /// `old_tag` or `new_tag` is not a tag this Vault could hold.
    InvalidTagName(String),
    /// At least one note carries the tag in a shape the rename cannot edit
    /// surgically. Nothing was written.
    UnsupportedShape(Vec<UnsupportedTagNote>),
    /// `expected_plan_hash` no longer matches the plan. Nothing was written.
    StalePlan,
    Write(WriteError),
}

impl From<WriteError> for TagRenameError {
    fn from(error: WriteError) -> Self {
        Self::Write(error)
    }
}

/// Plan the rename of `old_tag` to `new_tag` across the Vault `index` covers,
/// and, when `expected_plan_hash` is given, apply it if and only if that hash
/// is still the plan's fingerprint.
///
/// Renaming a tag renames everything nested beneath it, and `old_tag` need
/// not itself be carried by any note. Matching is case-insensitive, as tag
/// search is; `new_tag` must be written in lowercase.
pub fn rename_tag(
    vault_root: &Path,
    index: &VaultIndex,
    old_tag: &str,
    new_tag: &str,
    expected_plan_hash: Option<&str>,
) -> Result<TagRename, TagRenameError> {
    rename_tag_with_hook(
        vault_root,
        index,
        old_tag,
        new_tag,
        expected_plan_hash,
        |_| Ok(()),
    )
}

fn rename_tag_with_hook(
    vault_root: &Path,
    index: &VaultIndex,
    old_tag: &str,
    new_tag: &str,
    expected_plan_hash: Option<&str>,
    after_write: impl FnMut(usize) -> Result<(), WriteError>,
) -> Result<TagRename, TagRenameError> {
    let (old_tag, new_tag) = validated_names(old_tag, new_tag)?;
    let plan = plan(index, &old_tag, &new_tag)?;
    let Some(expected) = expected_plan_hash else {
        return Ok(plan.report);
    };
    if plan.report.plan_hash.as_deref() != Some(expected.trim()) {
        return Err(TagRenameError::StalePlan);
    }
    apply(vault_root, plan, after_write)
}

/// The two names, normalised. A leading `#` is accepted on either, since that
/// is how a tag is written in prose.
fn validated_names(old_tag: &str, new_tag: &str) -> Result<(String, String), TagRenameError> {
    let old = tag_name(old_tag, "old_tag")
        .map_err(TagRenameError::InvalidTagName)?
        .to_lowercase();
    let new = tag_name(new_tag, "new_tag").map_err(TagRenameError::InvalidTagName)?;
    if new.to_lowercase() != new {
        return Err(TagRenameError::InvalidTagName(format!(
            "new_tag '{new}' contains uppercase letters; tags are stored in lowercase, so write it in lowercase"
        )));
    }
    let new = new.to_string();
    // Renaming `a` to `a/b` would turn `a/x` into `a/b/x`, which is still under
    // `a`, so running it again would rename it again. A rename has to be
    // something a second run finds nothing left to do for.
    if new != old && tag_matches(&new, &old) {
        return Err(TagRenameError::InvalidTagName(format!(
            "new_tag '{new}' is nested under old_tag '{old}', so the renamed tags would still match old_tag"
        )));
    }
    Ok((old, new))
}

/// `raw` with one leading `#` removed, refused unless it is a tag the grammar
/// tag search accepts: letters, digits, `-`, `_`, and `/` between non-empty
/// segments.
fn tag_name<'a>(raw: &'a str, field: &str) -> Result<&'a str, String> {
    let name = raw.strip_prefix('#').unwrap_or(raw);
    if name.is_empty() {
        return Err(format!("{field} cannot be empty"));
    }
    if let Some(bad) = name.chars().find(|ch| !is_tag_char(*ch)) {
        return Err(format!(
            "{field} '{name}' contains '{bad}'; a tag may hold only letters, digits, '-', '_' and '/'"
        ));
    }
    if name.split('/').any(str::is_empty) {
        return Err(format!(
            "{field} '{name}' has an empty segment; '/' may only separate two parts of a tag"
        ));
    }
    Ok(name)
}

/// `tag` as written, renamed when it matches `old`, keeping the author's
/// spelling of whatever is nested beneath the renamed part. `None` when it
/// does not match.
fn renamed(tag: &str, old: &str, new: &str) -> Option<String> {
    if !tag_matches(&tag.to_lowercase(), old) {
        return None;
    }
    // Lowercasing can change a letter's byte length but never adds or removes
    // a `/`, so the segments line up between the two spellings.
    let segments = old.split('/').count();
    Some(match tag.splitn(segments + 1, '/').nth(segments) {
        Some(rest) => format!("{new}/{rest}"),
        None => new.to_string(),
    })
}

struct Plan {
    report: TagRename,
    rewrites: Vec<PlannedRewrite>,
}

struct PlannedRewrite {
    path: PathBuf,
    original_hash: String,
    content: String,
}

fn plan(index: &VaultIndex, old: &str, new: &str) -> Result<Plan, TagRenameError> {
    let mut notes = Vec::new();
    let mut rewrites = Vec::new();
    let mut unsupported = Vec::new();
    let mut already_tagged_notes = 0usize;
    for entry in index.ordered_entries() {
        // A note that cannot be opened carries no tag the index knows of.
        let Some(note) = read_note_text(&entry.path) else {
            continue;
        };
        let tags = extract_tags(&note.content);
        if tags.iter().any(|tag| tag_matches(tag, new)) {
            already_tagged_notes += 1;
        }
        if !tags.iter().any(|tag| tag_matches(tag, old)) {
            continue;
        }
        if !note.utf8 {
            unsupported.push(non_utf8_note(&entry));
            continue;
        }
        let content = note.content;
        // Renaming `domain/x` to `domain` turns `domain/x/x` into `domain/x`,
        // which is still under `domain/x`, so a second run would rename it
        // again. The names are fine on their own; it is this Vault's tags
        // that make them a rename that never settles.
        if new != old
            && let Some(unsettled) = tags
                .iter()
                .filter_map(|tag| renamed(tag, old, new))
                .find(|renamed| tag_matches(renamed, old))
        {
            return Err(TagRenameError::InvalidTagName(format!(
                "renaming '{old}' to '{new}' would turn a tag in '{}' into '{unsettled}', which is still under '{old}', so running the rename again would change it again",
                entry.relative_path
            )));
        }
        match rewrite_note(&entry, &content, &tags, old, new) {
            Ok(None) => {}
            Ok(Some(rewrite)) => {
                notes.push(TagRenameNote {
                    slug: entry.slug.clone(),
                    relative_path: entry.relative_path.clone(),
                    frontmatter: rewrite.frontmatter,
                    body: rewrite.body,
                    content_hash: content_hash(&content),
                });
                rewrites.push(PlannedRewrite {
                    path: entry.path.clone(),
                    original_hash: content_hash(&content),
                    content: rewrite.content,
                });
            }
            Err(reason) => unsupported.push(UnsupportedTagNote {
                relative_path: entry.relative_path.clone(),
                reason,
            }),
        }
    }
    if !unsupported.is_empty() {
        return Err(TagRenameError::UnsupportedShape(unsupported));
    }
    let plan_hash = fingerprint(old, new, &notes, &rewrites);
    Ok(Plan {
        report: TagRename {
            old_tag: old.to_string(),
            new_tag: new.to_string(),
            applied: false,
            frontmatter_notes: notes.iter().filter(|note| note.frontmatter).count(),
            body_notes: notes.iter().filter(|note| note.body).count(),
            notes,
            already_tagged_notes,
            plan_hash,
            affected_paths: Vec::new(),
        },
        rewrites,
    })
}

/// A hash of every edit in the plan, and of what each note held when it was
/// planned. Two plans share a fingerprint only if they would write the same
/// bytes over the same bytes.
fn fingerprint(
    old: &str,
    new: &str,
    notes: &[TagRenameNote],
    rewrites: &[PlannedRewrite],
) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    let mut canonical = format!("rename_tag\0{old}\0{new}\0");
    for (note, rewrite) in notes.iter().zip(rewrites) {
        canonical.push_str(&format!(
            "{}\0{}\0{}\0",
            note.relative_path,
            rewrite.original_hash,
            content_hash(&rewrite.content)
        ));
    }
    Some(content_hash(&canonical))
}

struct NoteRewrite {
    content: String,
    frontmatter: bool,
    body: bool,
}

/// The note's text with the tag renamed, or `None` when it already reads as
/// the rename would leave it. An `Err` is the reason the note cannot be edited
/// in place.
fn rewrite_note(
    entry: &NoteEntry,
    content: &str,
    tags: &std::collections::HashSet<String>,
    old: &str,
    new: &str,
) -> Result<Option<NoteRewrite>, String> {
    let mut rewritten = content.to_string();

    // The body first: its ranges sit after the frontmatter block, so editing
    // it from the end backwards leaves the block's own span where it was.
    let mut body = false;
    let inline: Vec<InlineTag> = inline_tags(content);
    for tag in inline.iter().rev() {
        let Some(replacement) = renamed(&tag.text, old, new) else {
            continue;
        };
        let Some(range) = tag.range.clone() else {
            return Err(format!(
                "its inline tag '#{}' is interrupted by a code span, so no single piece of text spells it",
                tag.text
            ));
        };
        if !replacement.contains('/') {
            return Err(format!(
                "its inline tag '#{}' would become '#{replacement}', which is not a tag in a note body because it has no '/'",
                tag.text
            ));
        }
        if replacement != tag.text {
            rewritten.replace_range(range, &replacement);
            body = true;
        }
    }

    let frontmatter = match frontmatter_span(content) {
        Some((start, end)) => {
            match rewrite_frontmatter_tags(
                &content[start..end],
                &entry.relative_path,
                TagOperation::Rename,
                |tags| renamed_tag_value(tags, old, new),
            )? {
                Some(block) => {
                    rewritten.replace_range(start..end, &block);
                    true
                }
                None => false,
            }
        }
        None => false,
    };

    // The backstop: read the result back the way the index will, and require
    // exactly the tags the rename promised. A tag the edits above could not
    // reach, such as one in a frontmatter block that is not valid YAML, leaves
    // the old name behind and fails here rather than being half-renamed.
    let promised: BTreeSet<String> = tags
        .iter()
        .map(|tag| renamed(tag, old, new).unwrap_or_else(|| tag.clone()))
        .collect();
    let read_back: BTreeSet<String> = extract_tags(&rewritten).into_iter().collect();
    if read_back != promised {
        return Err(
            "renaming its tags in place would not leave it carrying the renamed tags; check its frontmatter parses as YAML"
                .to_string(),
        );
    }
    if rewritten == content {
        return Ok(None);
    }
    Ok(Some(NoteRewrite {
        content: rewritten,
        frontmatter,
        body,
    }))
}

/// The frontmatter block with its `tags` replaced by what `change` makes of
/// them, or `None` when `change` has nothing to change.
///
/// The edit goes through the shared in-place editor, which writes a list in
/// the shape the author used but writes its items its own way. So before
/// trusting it with the change, it is handed the list unchanged: if that does
/// not reproduce the block byte for byte, the list is written in a way the
/// editor would reformat (extra spacing, quotes it would not use, a comment on
/// the line), and the note is refused rather than restyled.
fn rewrite_frontmatter_tags(
    block: &str,
    relative_path: &str,
    operation: TagOperation,
    change: impl FnOnce(&Value) -> Option<Value>,
) -> Result<Option<String>, String> {
    let Ok(Value::Object(properties)) = serde_yaml_ng::from_str::<Value>(block) else {
        // Not a mapping, so the index read no tags from it through YAML. The
        // backstop decides whether any reached it another way.
        return Ok(None);
    };
    let Some(tags) = properties.get("tags") else {
        return Ok(None);
    };
    let Some(changed_tags) = change(tags) else {
        return Ok(None);
    };
    let edit = |value: &Value| {
        let mut updates = Map::new();
        updates.insert("tags".to_string(), value.clone());
        match edit_frontmatter_block(block, &updates, relative_path) {
            Ok(FrontmatterEdit::Block(block)) => Ok(block),
            Ok(FrontmatterEdit::Empty) => Err("its frontmatter would be emptied".to_string()),
            Err(error) => Err(format!(
                "its frontmatter cannot be edited in place: {}",
                write_error_message(&error)
            )),
        }
    };
    if edit(tags)? != block {
        return Err(format!(
            "its frontmatter tags are formatted in a way the {} cannot keep, such as quoted items, extra spaces, or a comment on the list",
            operation.noun()
        ));
    }
    edit(&changed_tags).map(Some)
}

fn write_error_message(error: &WriteError) -> &str {
    match error {
        WriteError::Conflict(message)
        | WriteError::InvalidInput(message)
        | WriteError::Io(message) => message,
        WriteError::LinkRewriteUnsupported(_) => "a linking note cannot be rewritten",
    }
}

/// `tags` with every matching item renamed, or `None` when nothing matches.
///
/// An item the rename turns into a tag the list already carries is dropped, so
/// a note ends up with the target once; the earlier of the two keeps its
/// place. Items the rename does not produce are never dropped, duplicates
/// included, since they are not this operation's business.
fn renamed_tag_value(tags: &Value, old: &str, new: &str) -> Option<Value> {
    match tags {
        Value::String(tag) => renamed_item(tag, old, new).map(Value::String),
        Value::Array(items) => {
            let renamed_items: Vec<Option<String>> = items
                .iter()
                .map(|item| item.as_str().and_then(|tag| renamed_item(tag, old, new)))
                .collect();
            if renamed_items.iter().all(Option::is_none) {
                return None;
            }
            let produced: BTreeSet<String> = renamed_items
                .iter()
                .flatten()
                .map(|tag| normalized_item(tag))
                .collect();
            let mut seen = BTreeSet::new();
            let mut out = Vec::with_capacity(items.len());
            for (item, renamed) in items.iter().zip(renamed_items) {
                let value = renamed.map(Value::String).unwrap_or_else(|| item.clone());
                if let Some(tag) = value.as_str() {
                    let normalized = normalized_item(tag);
                    if produced.contains(&normalized) && !seen.insert(normalized) {
                        continue;
                    }
                }
                out.push(value);
            }
            Some(Value::Array(out))
        }
        _ => None,
    }
}

/// A frontmatter item the way the index normalises it.
fn normalized_item(tag: &str) -> String {
    tag.trim().trim_start_matches('#').to_lowercase()
}

/// One frontmatter item renamed, keeping any `#` and surrounding whitespace
/// the author wrote around the tag itself.
fn renamed_item(item: &str, old: &str, new: &str) -> Option<String> {
    let trimmed_start = item.trim_start();
    let core_start = item.len() - trimmed_start.trim_start_matches('#').len();
    let core_end = item.trim_end().len();
    if core_start >= core_end {
        return None;
    }
    let core = &item[core_start..core_end];
    let replacement = renamed(core, old, new)?;
    Some(format!(
        "{}{replacement}{}",
        &item[..core_start],
        &item[core_end..]
    ))
}

fn apply(
    vault_root: &Path,
    plan: Plan,
    after_write: impl FnMut(usize) -> Result<(), WriteError>,
) -> Result<TagRename, TagRenameError> {
    let affected_paths = write_rewrites(
        vault_root,
        &plan.rewrites,
        TagOperation::Rename,
        after_write,
    )?;
    let mut report = plan.report;
    for (note, rewrite) in report.notes.iter_mut().zip(&plan.rewrites) {
        note.content_hash = content_hash(&rewrite.content);
    }
    report.applied = true;
    report.affected_paths = affected_paths;
    Ok(report)
}

/// Which Vault-wide tag operation a shared step is serving, for its messages.
#[derive(Clone, Copy)]
enum TagOperation {
    Rename,
    Delete,
}

impl TagOperation {
    fn noun(self) -> &'static str {
        match self {
            Self::Rename => "rename",
            Self::Delete => "delete",
        }
    }

    fn editing(self) -> &'static str {
        match self {
            Self::Rename => "renaming its tags",
            Self::Delete => "removing a tag from it",
        }
    }
}

enum ApplyError {
    /// A planned note changed since it was read.
    Stale,
    Write(WriteError),
}

impl From<ApplyError> for TagRenameError {
    fn from(error: ApplyError) -> Self {
        match error {
            ApplyError::Stale => Self::StalePlan,
            ApplyError::Write(error) => Self::Write(error),
        }
    }
}

impl From<ApplyError> for TagDeleteError {
    fn from(error: ApplyError) -> Self {
        match error {
            ApplyError::Stale => Self::StalePlan,
            ApplyError::Write(error) => Self::Write(error),
        }
    }
}

/// Write every planned note, or none of them. Returns the paths written.
fn write_rewrites(
    vault_root: &Path,
    rewrites: &[PlannedRewrite],
    operation: TagOperation,
    mut after_write: impl FnMut(usize) -> Result<(), WriteError>,
) -> Result<Vec<PathBuf>, ApplyError> {
    // The plan was read under the same lock this write holds, but a person
    // editing the Vault directly is not bound by it. A note that moved on
    // since it was read is not overwritten with text built from the old copy.
    //
    // This front-loaded pass answers "is the whole plan still current?" once,
    // so a rename that is going to be refused is refused before anything is
    // written. It is not the protection: its verdict is a moment old by the
    // time the last note in the loop below is written. Each rewrite carries
    // its own `original_hash` into the commit, and the journal checks that
    // one per note at the moment it writes it (#321).
    for rewrite in rewrites {
        let current = fs::read_to_string(&rewrite.path).map_err(|error| {
            ApplyError::Write(WriteError::Io(format!(
                "failed to re-read '{}' before {}: {error}",
                rewrite.path.display(),
                operation.editing()
            )))
        })?;
        if content_hash(&current) != rewrite.original_hash {
            return Err(ApplyError::Stale);
        }
    }

    let mut journal = MutationJournal::new(vault_root);
    let mut affected_paths = Vec::with_capacity(rewrites.len());
    for (position, rewrite) in rewrites.iter().enumerate() {
        let written = journal
            .apply_rewrites(vec![TextRewrite {
                path: rewrite.path.clone(),
                original_hash: rewrite.original_hash.clone(),
                content: rewrite.content.clone(),
            }])
            .and_then(|written| after_write(position).map(|()| written));
        match written {
            Ok(written) => affected_paths.extend(written),
            Err(error) => return Err(ApplyError::Write(journal.rollback(error))),
        }
    }
    Ok(affected_paths)
}

/// The index reads a note that is not valid UTF-8 lossily, so it may well
/// report the tag. It cannot be rewritten without replacing the bytes that are
/// not text, so it is refused when it carries the tag and skipped otherwise.
fn non_utf8_note(entry: &NoteEntry) -> UnsupportedTagNote {
    UnsupportedTagNote {
        relative_path: entry.relative_path.clone(),
        reason: "the note is not valid UTF-8 text".to_string(),
    }
}

/// A delete that was planned, or planned and applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagDelete {
    /// The tag being deleted, normalised: no `#`, lowercase.
    pub tag: String,
    /// Whether this call wrote the notes below. A plan never does.
    pub applied: bool,
    /// Every note whose frontmatter loses the tag, in path order.
    pub notes: Vec<TagDeleteNote>,
    /// The plan's fingerprint. `None` when no note carries the tag.
    pub plan_hash: Option<String>,
    /// Absolute paths this call wrote. Empty for a plan.
    pub affected_paths: Vec<PathBuf>,
}

/// One note the delete changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagDeleteNote {
    pub slug: String,
    pub relative_path: String,
    /// The note's content hash once this call returns: its current hash for a
    /// plan, the rewritten note's hash once applied.
    pub content_hash: String,
}

/// A tag nested under the one being deleted, and how many notes carry it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedTag {
    pub tag: String,
    pub notes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagDeleteError {
    /// `tag` is not a tag this Vault could hold.
    InvalidTagName(String),
    /// Notes carry a tag nested under the one being deleted, and tag search
    /// would keep finding them under it. Nothing was written.
    NestedTags(Vec<NestedTag>),
    /// Notes, by relative path, that carry the tag inline in their body. A
    /// delete never edits prose. Nothing was written.
    InlineUse(Vec<String>),
    /// At least one note carries the tag in a shape the delete cannot edit
    /// surgically. Nothing was written.
    UnsupportedShape(Vec<UnsupportedTagNote>),
    /// `expected_plan_hash` no longer matches the plan. Nothing was written.
    StalePlan,
    Write(WriteError),
}

impl From<WriteError> for TagDeleteError {
    fn from(error: WriteError) -> Self {
        Self::Write(error)
    }
}

/// Plan the removal of `tag` from every frontmatter `tags` value in the Vault
/// `index` covers, and, when `expected_plan_hash` is given, apply it if and
/// only if that hash is still the plan's fingerprint.
///
/// Only the exact tag goes. The delete is refused while any note carries a tag
/// nested under it or carries it inline, so that once it applies, a tag search
/// for it finds nothing. A list it empties stays behind as `tags: []`.
pub fn delete_tag(
    vault_root: &Path,
    index: &VaultIndex,
    tag: &str,
    expected_plan_hash: Option<&str>,
) -> Result<TagDelete, TagDeleteError> {
    let tag = tag_name(tag, "tag")
        .map_err(TagDeleteError::InvalidTagName)?
        .to_lowercase();
    let (mut report, rewrites) = plan_delete(index, &tag)?;
    let Some(expected) = expected_plan_hash else {
        return Ok(report);
    };
    if report.plan_hash.as_deref() != Some(expected.trim()) {
        return Err(TagDeleteError::StalePlan);
    }
    report.affected_paths =
        write_rewrites(vault_root, &rewrites, TagOperation::Delete, |_| Ok(()))?;
    for (note, rewrite) in report.notes.iter_mut().zip(&rewrites) {
        note.content_hash = content_hash(&rewrite.content);
    }
    report.applied = true;
    Ok(report)
}

fn plan_delete(
    index: &VaultIndex,
    tag: &str,
) -> Result<(TagDelete, Vec<PlannedRewrite>), TagDeleteError> {
    let mut nested: BTreeMap<String, usize> = BTreeMap::new();
    let mut inline = Vec::new();
    let mut unsupported = Vec::new();
    let mut notes = Vec::new();
    let mut rewrites = Vec::new();
    for entry in index.ordered_entries() {
        let Some(note) = read_note_text(&entry.path) else {
            continue;
        };
        let tags = extract_tags(&note.content);
        for carried in tags.iter().filter(|carried| carried.as_str() != tag) {
            if tag_matches(carried, tag) {
                *nested.entry(carried.clone()).or_default() += 1;
            }
        }
        if !tags.contains(tag) {
            continue;
        }
        if inline_tags(&note.content)
            .iter()
            .any(|inline| inline.text.to_lowercase() == tag)
        {
            inline.push(entry.relative_path.clone());
            continue;
        }
        if !note.utf8 {
            unsupported.push(non_utf8_note(&entry));
            continue;
        }
        match delete_from_note(&entry, &note.content, &tags, tag) {
            Ok(rewritten) => {
                let original_hash = content_hash(&note.content);
                notes.push(TagDeleteNote {
                    slug: entry.slug.clone(),
                    relative_path: entry.relative_path.clone(),
                    content_hash: original_hash.clone(),
                });
                rewrites.push(PlannedRewrite {
                    path: entry.path.clone(),
                    original_hash,
                    content: rewritten,
                });
            }
            Err(reason) => unsupported.push(UnsupportedTagNote {
                relative_path: entry.relative_path.clone(),
                reason,
            }),
        }
    }
    // Nested tags first: clearing a branch bottom-up is the larger job, and a
    // nested tag's own inline uses are the next delete's business.
    if !nested.is_empty() {
        return Err(TagDeleteError::NestedTags(
            nested
                .into_iter()
                .map(|(tag, notes)| NestedTag { tag, notes })
                .collect(),
        ));
    }
    if !inline.is_empty() {
        return Err(TagDeleteError::InlineUse(inline));
    }
    if !unsupported.is_empty() {
        return Err(TagDeleteError::UnsupportedShape(unsupported));
    }
    let plan_hash = (!notes.is_empty()).then(|| {
        let mut canonical = format!("delete_tag\0{tag}\0");
        for (note, rewrite) in notes.iter().zip(&rewrites) {
            canonical.push_str(&format!(
                "{}\0{}\0{}\0",
                note.relative_path,
                rewrite.original_hash,
                content_hash(&rewrite.content)
            ));
        }
        content_hash(&canonical)
    });
    Ok((
        TagDelete {
            tag: tag.to_string(),
            applied: false,
            notes,
            plan_hash,
            affected_paths: Vec::new(),
        },
        rewrites,
    ))
}

/// The note's text with `tag` removed from its frontmatter. An `Err` is the
/// reason the note cannot be edited in place.
fn delete_from_note(
    entry: &NoteEntry,
    content: &str,
    tags: &HashSet<String>,
    tag: &str,
) -> Result<String, String> {
    let mut rewritten = content.to_string();
    if let Some((start, end)) = frontmatter_span(content)
        && let Some(block) = rewrite_frontmatter_tags(
            &content[start..end],
            &entry.relative_path,
            TagOperation::Delete,
            |tags| without_tag(tags, tag),
        )?
    {
        rewritten.replace_range(start..end, &block);
    }
    // The same backstop as a rename: the index must read back every tag the
    // note had except this one. A tag the edit could not reach, such as one in
    // a frontmatter block that is not valid YAML, fails here.
    let promised: BTreeSet<String> = tags
        .iter()
        .filter(|carried| carried.as_str() != tag)
        .cloned()
        .collect();
    let read_back: BTreeSet<String> = extract_tags(&rewritten).into_iter().collect();
    if read_back != promised {
        return Err(
            "removing the tag in place would not leave it without the tag; check its frontmatter parses as YAML"
                .to_string(),
        );
    }
    Ok(rewritten)
}

/// `tags` without any item that names `tag`, or `None` when none does. A
/// scalar is a one-item list, so removing it leaves an empty list.
fn without_tag(tags: &Value, tag: &str) -> Option<Value> {
    let names = |item: &Value| {
        item.as_str()
            .is_some_and(|item| normalized_item(item) == tag)
    };
    match tags {
        Value::String(_) if names(tags) => Some(Value::Array(Vec::new())),
        Value::Array(items) if items.iter().any(names) => Some(Value::Array(
            items.iter().filter(|item| !names(item)).cloned().collect(),
        )),
        _ => None,
    }
}

#[cfg(test)]
pub(super) fn rename_tag_with_failure(
    vault_root: &Path,
    index: &VaultIndex,
    old_tag: &str,
    new_tag: &str,
    expected_plan_hash: Option<&str>,
    after_write: impl FnMut(usize) -> Result<(), WriteError>,
) -> Result<TagRename, TagRenameError> {
    rename_tag_with_hook(
        vault_root,
        index,
        old_tag,
        new_tag,
        expected_plan_hash,
        after_write,
    )
}
