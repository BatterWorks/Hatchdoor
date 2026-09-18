//! Vault-wide tag rename (#242).
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

use std::collections::BTreeSet;
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
    let old = tag_name(old_tag, "old_tag")?.to_lowercase();
    let new = tag_name(new_tag, "new_tag")?;
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
fn tag_name<'a>(raw: &'a str, field: &str) -> Result<&'a str, TagRenameError> {
    let name = raw.strip_prefix('#').unwrap_or(raw);
    if name.is_empty() {
        return Err(TagRenameError::InvalidTagName(format!(
            "{field} cannot be empty"
        )));
    }
    if let Some(bad) = name.chars().find(|ch| !is_tag_char(*ch)) {
        return Err(TagRenameError::InvalidTagName(format!(
            "{field} '{name}' contains '{bad}'; a tag may hold only letters, digits, '-', '_' and '/'"
        )));
    }
    if name.split('/').any(str::is_empty) {
        return Err(TagRenameError::InvalidTagName(format!(
            "{field} '{name}' has an empty segment; '/' may only separate two parts of a tag"
        )));
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
        let bytes = fs::read(&entry.path).map_err(|error| {
            WriteError::Io(format!(
                "failed to read note '{}' for tag rename: {error}",
                entry.relative_path
            ))
        })?;
        let content = match String::from_utf8(bytes) {
            Ok(content) => content,
            Err(error) => {
                // The index reads a note like this lossily, so it may well
                // report the tag here. It cannot be rewritten without
                // replacing the bytes that are not text, so it is refused
                // when it matters and skipped when it does not.
                let lossy = String::from_utf8_lossy(error.as_bytes()).into_owned();
                let tags = extract_tags(&lossy);
                if tags.iter().any(|tag| tag_matches(tag, new)) {
                    already_tagged_notes += 1;
                }
                if tags.iter().any(|tag| tag_matches(tag, old)) {
                    unsupported.push(UnsupportedTagNote {
                        relative_path: entry.relative_path.clone(),
                        reason: "the note is not valid UTF-8 text".to_string(),
                    });
                }
                continue;
            }
        };
        let tags = extract_tags(&content);
        if tags.iter().any(|tag| tag_matches(tag, new)) {
            already_tagged_notes += 1;
        }
        if !tags.iter().any(|tag| tag_matches(tag, old)) {
            continue;
        }
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
            match rewrite_frontmatter_tags(&content[start..end], old, new, &entry.relative_path)? {
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

/// The frontmatter block with its `tags` renamed, or `None` when `tags` holds
/// nothing to rename.
///
/// The edit goes through the shared in-place editor, which writes a list in
/// the shape the author used but writes its items its own way. So before
/// trusting it with the rename, it is handed the list unchanged: if that does
/// not reproduce the block byte for byte, the list is written in a way the
/// editor would reformat (extra spacing, quotes it would not use, a comment on
/// the line), and the note is refused rather than restyled.
fn rewrite_frontmatter_tags(
    block: &str,
    old: &str,
    new: &str,
    relative_path: &str,
) -> Result<Option<String>, String> {
    let Ok(Value::Object(properties)) = serde_yaml_ng::from_str::<Value>(block) else {
        // Not a mapping, so the index read no tags from it through YAML. The
        // backstop decides whether any reached it another way.
        return Ok(None);
    };
    let Some(tags) = properties.get("tags") else {
        return Ok(None);
    };
    let Some(renamed_tags) = renamed_tag_value(tags, old, new) else {
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
        return Err(
            "its frontmatter tags are formatted in a way the rename cannot keep, such as quoted items, extra spaces, or a comment on the list"
                .to_string(),
        );
    }
    edit(&renamed_tags).map(Some)
}

fn write_error_message(error: &WriteError) -> &str {
    match error {
        WriteError::Conflict(message)
        | WriteError::InvalidInput(message)
        | WriteError::Io(message) => message,
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
    mut after_write: impl FnMut(usize) -> Result<(), WriteError>,
) -> Result<TagRename, TagRenameError> {
    // The plan was read under the same lock this write holds, but a person
    // editing the Vault directly is not bound by it. A note that moved on
    // since it was read is not overwritten with text built from the old copy.
    for rewrite in &plan.rewrites {
        let current = fs::read_to_string(&rewrite.path).map_err(|error| {
            WriteError::Io(format!(
                "failed to re-read '{}' before renaming its tags: {error}",
                rewrite.path.display()
            ))
        })?;
        if content_hash(&current) != rewrite.original_hash {
            return Err(TagRenameError::StalePlan);
        }
    }

    let mut journal = MutationJournal::new(vault_root);
    let mut affected_paths = Vec::with_capacity(plan.rewrites.len());
    for (position, rewrite) in plan.rewrites.iter().enumerate() {
        let written = journal
            .apply_rewrites(vec![TextRewrite {
                path: rewrite.path.clone(),
                content: rewrite.content.clone(),
            }])
            .and_then(|written| after_write(position).map(|()| written));
        match written {
            Ok(written) => affected_paths.extend(written),
            Err(error) => return Err(TagRenameError::Write(journal.rollback(error))),
        }
    }

    let mut report = plan.report;
    for (note, rewrite) in report.notes.iter_mut().zip(&plan.rewrites) {
        note.content_hash = content_hash(&rewrite.content);
    }
    report.applied = true;
    report.affected_paths = affected_paths;
    Ok(report)
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
