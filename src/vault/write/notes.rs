use std::fs;
use std::path::{Path, PathBuf};

use crate::cache::parse::content_hash;
use crate::vault::paths::{slugify, strip_md_extension};
use crate::vault::sections::NoteSections;
use crate::vault::types::{NoteEntry, VaultIndex};

use super::assets::asset_move_plan;
use super::frontmatter::{FrontmatterEdit, edit_frontmatter_block};
use super::fs_ops::{
    MutationJournal, atomic_create, atomic_write, atomic_write_if_unchanged, ensure_content_hash,
};
use super::paths::{
    create_parent_dir_inside_root, normalize_note_relative_path, resolve_new_note_path,
    unique_trash_relative_path,
};
use super::rewrites::{MovedTo, backlink_rewrite_plan, merge_rewrites, refuse_unrewritable};
use super::types::{AssetMove, MutationPhase, TextRewrite, WriteError, WriteOutcome};
use crate::cache::parse::frontmatter_span;

/// Where `replace_section` places the supplied content relative to the matched section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionMode {
    /// Replace the whole section (heading line through the body before the next same-or-higher heading).
    Replace,
    /// Insert the content immediately before the heading line, leaving the section intact.
    Before,
    /// Insert the content immediately after the section, leaving the section intact.
    After,
}

/// Text a write is about to put on disk, and the quality warnings that
/// describe what preparing it changed.
struct PreparedText {
    content: String,
    warnings: Vec<String>,
}

fn reject_nul(content: &str) -> Result<(), WriteError> {
    if content.contains('\0') {
        return Err(WriteError::InvalidInput(
            "note content cannot contain NUL bytes".to_string(),
        ));
    }
    Ok(())
}

/// `content` with every CRLF and lone CR turned into LF.
fn to_lf(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

/// Whether `content` ends in a line break. A lone CR counts, as it does in
/// CommonMark, so a separator is never added after one.
fn ends_with_line_break(content: &str) -> bool {
    content.ends_with(['\n', '\r'])
}

/// Normalise the content of a whole-content write (`create_note`,
/// `update_note`): CRLF and CR become LF and a final newline is added. ADR-22
/// allows this only for a write that replaces the whole note; the partial
/// writes use `prepare_inserted_text` instead.
fn prepare_note_content(content: &str) -> Result<PreparedText, WriteError> {
    reject_nul(content)?;

    let mut warnings = Vec::new();
    let mut normalized = to_lf(content);
    if normalized != content {
        warnings.push("normalized CRLF/CR line endings to LF".to_string());
    }
    if !normalized.is_empty() && !normalized.ends_with('\n') {
        normalized.push('\n');
        warnings.push("added final newline".to_string());
    }
    warnings.extend(frontmatter_warnings(&normalized));

    Ok(PreparedText {
        content: normalized,
        warnings,
    })
}

const BREAK_BEFORE_WARNING: &str = "added a line break before the supplied text";
const BREAK_AFTER_WARNING: &str = "added a line break after the supplied text";

/// The line ending a partial write gives the text it inserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    /// Whichever of CRLF and lone LF `content` uses more often, and LF on a
    /// tie or when it has no line breaks. The frontend's `detectLineEnding`
    /// applies the same rule.
    fn of(content: &str) -> Self {
        let crlf = content.matches("\r\n").count();
        let lone_lf = content.matches('\n').count() - crlf;
        if crlf > lone_lf { Self::Crlf } else { Self::Lf }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::Crlf => "\r\n",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::Crlf => "CRLF",
        }
    }
}

/// Prepare the text a partial write inserts into an existing note (ADR-22).
/// Only the caller's text is touched: its line breaks, in whatever form they
/// arrived, are written in the note's own `line_ending`. The note itself is
/// never normalised, so the warnings describe the supplied text alone.
fn prepare_inserted_text(text: &str, line_ending: LineEnding) -> Result<PreparedText, WriteError> {
    reject_nul(text)?;
    let conformed = match line_ending {
        LineEnding::Lf => to_lf(text),
        other => to_lf(text).replace('\n', other.as_str()),
    };
    let mut warnings = Vec::new();
    if conformed != text {
        warnings.push(format!(
            "converted line endings in the supplied text to {} to match the note",
            line_ending.name()
        ));
    }
    Ok(PreparedText {
        content: conformed,
        warnings,
    })
}

/// The frontmatter quality warnings for a note a partial write produced. The
/// check reads an LF view so a CRLF note is checked like any other; the note
/// on disk is not changed by it.
fn partial_write_frontmatter_warnings(content: &str) -> Vec<String> {
    if !content.contains('\r') {
        return frontmatter_warnings(content);
    }
    frontmatter_warnings(&to_lf(content))
}

fn frontmatter_warnings(content: &str) -> Vec<String> {
    let Some(rest) = content.strip_prefix("---\n") else {
        return Vec::new();
    };

    let mut warnings = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut closed = false;
    for line in rest.lines() {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some((key, _)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        if !seen.insert(key.to_string()) {
            warnings.push(format!("frontmatter has duplicate key: {key}"));
        }
    }
    if !closed {
        warnings.push("frontmatter opening marker has no closing marker".to_string());
    }
    warnings
}

/// The priority order `VaultIndex::build_catalog_with_config` (`vault/index.rs`)
/// implicitly assigns slugs in: `markdown_paths.sort()` then a stable
/// `sort_by_cached_key(is_layered)` puts every default-surface note ahead of
/// every layered one (preserving path order within each group), so on a
/// title collision a default-surface note always claims the contested slug
/// first. Comparing `(is_layered, Path)` tuples lexicographically reproduces
/// that exact order: `false < true` puts default-surface first, and `Path`'s
/// component-wise `Ord` matches how the real build sorts `PathBuf`s.
///
/// Crucially, `path` here must still carry the `.md` extension: `markdown_paths.sort()`
/// runs on the raw `WalkDir` paths *before* any extension-stripping happens
/// (that only happens per-entry, later, inside the loop), so the real sort
/// compares e.g. `"Home.md"` against `"Home!!.md"`, not `"Home"` against
/// `"Home!!"`. These two orderings are not equivalent — `.` (0x2E) sorts
/// before common title punctuation like `!`, so stripping the extension
/// before comparing can reverse the outcome. A plain string compare of the
/// extension-free `relative_path` would be doubly wrong for the same reason
/// (see the module's `move_or_rename_note_disambiguates_a_slug_collision_against_a_different_note`
/// test in `write/tests.rs`, which exists specifically to pin this down).
fn slug_priority(catalog: &VaultIndex, relative_path: &str, path: &Path) -> (bool, PathBuf) {
    (
        catalog.layers.layer_for(relative_path).is_some(),
        path.to_path_buf(),
    )
}

/// The slug a fresh `VaultIndex::build_with_config` would assign to a note
/// whose extension-free relative path is `relative_path` (and whose real,
/// extension-bearing on-disk path is `path`), computed from a catalog already
/// fetched under this Vault's mutation lock instead of re-walking the Vault.
///
/// This is deliberately not a plain `unique_slug` occupancy check
/// (`vault/paths.rs`): a real index build assigns slugs in priority order
/// (`slug_priority` above), so an already-catalogued note only "blocks" a
/// candidate slug for this note if it has *higher* priority — a
/// lower-priority occupant would itself be bumped in a real rebuild once this
/// note claims the slug first, and never gets to hold it against us. Checking
/// literal `by_slug` occupancy while ignoring priority (as `unique_slug` does)
/// is only correct for `index.rs`'s own build loop, which already visits
/// entries in priority order, so nothing lower-priority has been inserted yet
/// when a given entry is assigned.
///
/// `exclude_slug`, when set, is the note's own pre-existing slug: without it,
/// recomputing a rename/move's slug would collide with the note's own
/// still-present entry in `by_slug` whenever the new title slugifies back to
/// the same value (e.g. renaming "Home" to "home").
fn slug_for_relative_path(
    catalog: &VaultIndex,
    relative_path: &str,
    path: &Path,
    exclude_slug: Option<&str>,
) -> String {
    let stem = relative_path.rsplit('/').next().unwrap_or(relative_path);
    let mut base = slugify(stem);
    if base.is_empty() {
        base = "untitled".to_string();
    }
    let priority = slug_priority(catalog, relative_path, path);

    let mut idx = 1usize;
    loop {
        let candidate = if idx == 1 {
            base.clone()
        } else {
            format!("{base}-{idx}")
        };
        let blocked = catalog.by_slug.get(&candidate).is_some_and(|entry| {
            Some(entry.slug.as_str()) != exclude_slug
                && slug_priority(catalog, &entry.relative_path, &entry.path) < priority
        });
        if !blocked {
            return candidate;
        }
        idx += 1;
    }
}

pub fn create_note(
    vault_root: &Path,
    relative_path: &str,
    content: &str,
    overwrite: bool,
    catalog: &VaultIndex,
) -> Result<WriteOutcome, WriteError> {
    let path = resolve_new_note_path(vault_root, relative_path)?;
    let normalized = normalize_note_relative_path(relative_path)?;
    let already_exists = || WriteError::Conflict(format!("Note already exists: {normalized}"));
    if path.exists() && !overwrite {
        return Err(already_exists());
    }

    create_parent_dir_inside_root(vault_root, &path, "note")?;

    let prepared = prepare_note_content(content)?;
    if overwrite {
        atomic_write(&path, &prepared.content)?;
    } else {
        // The check above is only a fast answer: a file created at this path
        // since then, by anything outside Hatchdoor, is refused at the commit
        // itself rather than replaced.
        atomic_create(&path, &prepared.content).map_err(|error| match error {
            WriteError::Conflict(_) => already_exists(),
            other => other,
        })?;
    }
    let relative_without_ext = strip_md_extension(&normalized).to_string();
    let slug = slug_for_relative_path(catalog, &relative_without_ext, &path, None);
    Ok(WriteOutcome {
        slug: Some(slug),
        relative_path: Some(relative_without_ext),
        content_hash: Some(content_hash(&prepared.content)),
        quality_warnings: prepared.warnings,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
        affected_paths: vec![path.clone()],
    })
}

/// Where a note write addressed by path lands, checked the way `create_note`
/// checks it: the Vault-relative path without `.md`, and whether a file is
/// already there. An uploaded note arrives with a path rather than a slug, so
/// this is how it finds out before the bytes do whether it would create a
/// note or collide with one (#303).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteTarget {
    pub relative_path: String,
    pub exists: bool,
    /// The path as `create_note` writes it, `.md` included.
    normalized: String,
}

pub fn note_target(vault_root: &Path, relative_path: &str) -> Result<NoteTarget, WriteError> {
    let path = resolve_new_note_path(vault_root, relative_path)?;
    let normalized = normalize_note_relative_path(relative_path)?;
    Ok(NoteTarget {
        relative_path: strip_md_extension(&normalized).to_string(),
        exists: path.exists(),
        normalized,
    })
}

/// The refusal `create_note` gives a note that already exists, for a caller
/// that found it with [`note_target`] before writing anything.
pub fn note_exists_conflict(target: &NoteTarget) -> WriteError {
    WriteError::Conflict(format!("Note already exists: {}", target.normalized))
}

/// `update_note`'s hash check alone, for a caller that wants its refusal
/// before it has the new content.
pub fn check_note_content_hash(entry: &NoteEntry, expected: &str) -> Result<(), WriteError> {
    ensure_content_hash(entry, expected)
}

pub fn update_note(
    entry: &NoteEntry,
    content: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let prepared = prepare_note_content(content)?;
    atomic_write_if_unchanged(&entry.path, &prepared.content, expected_content_hash)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&prepared.content)),
        quality_warnings: prepared.warnings,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
        affected_paths: vec![entry.path.clone()],
    })
}

/// Shallow top-level YAML merge into one note's frontmatter: every key in
/// `updates` replaces (or creates) its top-level frontmatter value wholesale —
/// nested mappings are not merged recursively — while keys `updates` does not
/// mention survive untouched. A `null` value deletes the key. A note with no
/// block gets one created, and deleting a note's last frontmatter key removes
/// the now-empty block entirely.
///
/// The block is edited in place rather than regenerated (ADR-22): every byte
/// that does not belong to a named key comes back exactly as the author wrote
/// it — key order, one-line versus multi-line lists, indentation, quoting,
/// comments, and blank lines — and so does the Markdown body outside the
/// block. See `write/frontmatter.rs` for the editing rules and the two
/// refusals they add. Reuses the canonical cache-layer frontmatter parsing
/// (`cache/parse.rs`) so reads and writes agree on what the block is.
pub fn update_note_frontmatter(
    entry: &NoteEntry,
    updates: serde_json::Map<String, serde_json::Value>,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    if updates.is_empty() {
        return Err(WriteError::InvalidInput(
            "update_frontmatter needs at least one top-level key".to_string(),
        ));
    }
    ensure_content_hash(entry, expected_content_hash)?;
    let content = read_note(entry)?;
    let span = frontmatter_span(&content);
    let had_frontmatter = span.is_some();
    // Surface the same frontmatter quality contract as the sibling note
    // primitives: a block can carry a duplicate key that every YAML reader
    // collapses (last one wins), so that loss is reported as a warning rather
    // than dropped silently. Naming the duplicated key is refused outright;
    // this warning is what a caller editing some *other* key still learns.
    let warnings = frontmatter_warnings(&content);
    let block = &content[span.map(|(start, end)| start..end).unwrap_or(0..0)];
    let edited = edit_frontmatter_block(block, &updates, &entry.relative_path)?;

    let updated = match edited {
        FrontmatterEdit::Empty => {
            if !had_frontmatter {
                return Err(WriteError::InvalidInput(
                    "update_frontmatter cannot create an empty frontmatter block; only null values were supplied".to_string(),
                ));
            }
            // Every key was deleted: strip the whole block instead of leaving
            // an empty `---\n---` pair behind. `end + 4` steps over the closing
            // "\n---", and the rest of the line it sits on goes with it: any
            // trailing spaces, then the newline that ends it. That newline
            // terminates the marker rather than opening the body, and leaving
            // it behind gave every stripped note a blank first line. A file
            // whose closing marker is its last bytes has no line ending to
            // drop, and a CRLF file has two bytes of it rather than one.
            let body_start = span.map_or(0, |(_, end)| {
                let rest = &content[end + 4..];
                let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                let line_ending = match &rest[spaces..] {
                    rest if rest.starts_with("\r\n") => 2,
                    rest if rest.starts_with('\n') => 1,
                    _ => 0,
                };
                end + 4 + spaces + line_ending
            });
            content[body_start..].to_string()
        }
        // Rewrite exactly the inner region so the opening/closing markers and
        // everything after them keep their original bytes. The edited block
        // carries no trailing newline, matching the span's own convention.
        FrontmatterEdit::Block(edited) => match span {
            Some((start, end)) => {
                format!("{}{}{}", &content[..start], edited, &content[end..])
            }
            // A note with no block yet gives the editor no line ending to copy,
            // so the note's own is applied here. Without it, creating a block
            // on a CRLF note leaves the file with mixed endings, which is the
            // thing the editor's own line-ending handling exists to avoid.
            None => match content.contains("\r\n") {
                true => {
                    let edited = edited.replace('\n', "\r\n");
                    format!("---\r\n{edited}\r\n---\r\n{content}")
                }
                false => format!("---\n{edited}\n---\n{content}"),
            },
        },
    };

    atomic_write_if_unchanged(&entry.path, &updated, expected_content_hash)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&updated)),
        quality_warnings: warnings,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
        affected_paths: vec![entry.path.clone()],
    })
}

/// Append text to a note. The existing bytes are kept as they are; the
/// appended text uses the note's own line ending, is separated from a last
/// line that has no line break, and ends with a line break (ADR-22).
pub fn append_note(
    entry: &NoteEntry,
    content: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let current = read_note(entry)?;
    let line_ending = LineEnding::of(&current);
    let appended = prepare_inserted_text(content, line_ending)?;
    let mut warnings = appended.warnings;
    // Appended text always ends with a line break, whatever the note did.
    let splice = Splice {
        block: &appended.content,
        line_ending,
        end_with_break: true,
    };
    let updated = splice.join(&current, "", &mut warnings);
    warnings.extend(partial_write_frontmatter_warnings(&updated));
    atomic_write_if_unchanged(&entry.path, &updated, expected_content_hash)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&updated)),
        quality_warnings: warnings,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
        affected_paths: vec![entry.path.clone()],
    })
}

pub fn edit_note(
    entry: &NoteEntry,
    old_string: &str,
    new_string: &str,
    expected_content_hash: &str,
    replace_all: bool,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    if old_string.is_empty() {
        return Err(WriteError::InvalidInput(
            "old_string cannot be empty".to_string(),
        ));
    }
    let current = read_note(entry)?;
    let matches = current.matches(old_string).count();
    match matches {
        0 => {
            return Err(WriteError::InvalidInput(format!(
                "old_string not found in note '{}'",
                entry.relative_path
            )));
        }
        count if count > 1 && !replace_all => {
            return Err(WriteError::Conflict(format!(
                "old_string is not unique in note '{}' ({count} matches); add surrounding context or pass replace_all",
                entry.relative_path
            )));
        }
        _ => {}
    }
    // Only the matched text changes (ADR-22): the replacement takes the
    // note's line ending, and nothing else in the note is normalised.
    let replacement = prepare_inserted_text(new_string, LineEnding::of(&current))?;
    let updated = if replace_all {
        current.replace(old_string, &replacement.content)
    } else {
        current.replacen(old_string, &replacement.content, 1)
    };
    let mut warnings = replacement.warnings;
    warnings.extend(partial_write_frontmatter_warnings(&updated));
    atomic_write_if_unchanged(&entry.path, &updated, expected_content_hash)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&updated)),
        quality_warnings: warnings,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
        affected_paths: vec![entry.path.clone()],
    })
}

pub fn replace_section(
    entry: &NoteEntry,
    heading: &str,
    mode: SectionMode,
    content: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let requested = heading.trim();
    if !requested.starts_with('#') {
        return Err(WriteError::InvalidInput(
            "heading must start with one or more '#' characters".to_string(),
        ));
    }
    let current = read_note(entry)?;
    let (start, end) = section_span(&current, requested, &entry.relative_path)?;
    let line_ending = LineEnding::of(&current);
    let block = prepare_inserted_text(content, line_ending)?;
    let mut warnings = block.warnings;
    let splice = Splice {
        block: &block.content,
        line_ending,
        end_with_break: ends_with_line_break(&current),
    };
    let updated = match mode {
        SectionMode::Replace => splice.join(&current[..start], &current[end..], &mut warnings),
        SectionMode::Before => splice.join(&current[..start], &current[start..], &mut warnings),
        SectionMode::After => splice.join(&current[..end], &current[end..], &mut warnings),
    };
    warnings.extend(partial_write_frontmatter_warnings(&updated));
    atomic_write_if_unchanged(&entry.path, &updated, expected_content_hash)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&updated)),
        quality_warnings: warnings,
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
        affected_paths: vec![entry.path.clone()],
    })
}

fn read_note(entry: &NoteEntry) -> Result<String, WriteError> {
    fs::read_to_string(&entry.path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read note '{}': {error}",
            entry.relative_path
        ))
    })
}

/// Byte range `[start, end)` covering the requested section, by the shared
/// section rule (`vault::sections`) the read core's section reads use too.
fn section_span(
    content: &str,
    requested: &str,
    relative_path: &str,
) -> Result<(usize, usize), WriteError> {
    let sections = NoteSections::scan(content);
    let matched: Vec<usize> = sections
        .headings()
        .iter()
        .enumerate()
        .filter(|(_, heading)| heading.line == requested)
        .map(|(idx, _)| idx)
        .collect();
    match matched.as_slice() {
        [] => Err(WriteError::InvalidInput(format!(
            "heading '{requested}' not found in note '{relative_path}'"
        ))),
        [idx] => {
            let span = sections.span(*idx);
            Ok((span.start, span.end))
        }
        more => Err(WriteError::Conflict(format!(
            "heading '{requested}' is not unique in note '{relative_path}' ({} matches)",
            more.len()
        ))),
    }
}

/// A block of supplied text `replace_section` or `append_note` places between
/// two untouched parts of a note.
struct Splice<'a> {
    block: &'a str,
    line_ending: LineEnding,
    /// Whether a block that ends the note must end with a line break.
    end_with_break: bool,
}

impl Splice<'_> {
    /// Join `prefix + block + suffix`, adding a line break in the note's own
    /// line ending wherever the block would otherwise glue onto an adjacent
    /// line, and reporting each one. A block that ends the note ends with a
    /// line break only when `end_with_break` says so. An empty block removes
    /// text and adds nothing. The block has been through
    /// `prepare_inserted_text`, so it never ends in a lone CR and checking for
    /// `\n` is enough.
    fn join(&self, prefix: &str, suffix: &str, warnings: &mut Vec<String>) -> String {
        let mut out = String::with_capacity(prefix.len() + self.block.len() + suffix.len() + 4);
        out.push_str(prefix);
        if !self.block.is_empty() {
            if !prefix.is_empty() && !ends_with_line_break(prefix) {
                out.push_str(self.line_ending.as_str());
                warnings.push(BREAK_BEFORE_WARNING.to_string());
            }
            out.push_str(self.block);
            let needs_break = !suffix.is_empty() || self.end_with_break;
            if needs_break && !self.block.ends_with('\n') {
                out.push_str(self.line_ending.as_str());
                warnings.push(BREAK_AFTER_WARNING.to_string());
            }
        }
        out.push_str(suffix);
        out
    }
}

pub fn move_or_rename_note(
    vault_root: &Path,
    index: &VaultIndex,
    entry: &NoteEntry,
    target_relative_path: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    move_or_rename_note_with_hook(
        vault_root,
        index,
        entry,
        target_relative_path,
        expected_content_hash,
        |_| Ok(()),
    )
}

fn move_or_rename_note_with_hook(
    vault_root: &Path,
    index: &VaultIndex,
    entry: &NoteEntry,
    target_relative_path: &str,
    expected_content_hash: &str,
    mut after_phase: impl FnMut(MutationPhase) -> Result<(), WriteError>,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let target_path = resolve_new_note_path(vault_root, target_relative_path)?;
    if target_path.exists() {
        return Err(WriteError::Conflict(format!(
            "Destination note already exists: {}",
            normalize_note_relative_path(target_relative_path)?
        )));
    }
    let target_without_ext =
        strip_md_extension(&normalize_note_relative_path(target_relative_path)?).to_string();
    let slug = slug_for_relative_path(index, &target_without_ext, &target_path, Some(&entry.slug));
    let mut unrewritable = Vec::new();
    let backlink_rewrites = backlink_rewrite_plan(
        index,
        &entry.slug,
        Some(MovedTo {
            new_target: &target_without_ext,
            destination: target_path.as_path(),
        }),
        &mut unrewritable,
    );
    let (asset_moves, asset_rewrites) = asset_move_plan(
        vault_root,
        index,
        entry,
        &target_path,
        false,
        &backlink_rewrites,
        &mut unrewritable,
    )?;
    refuse_unrewritable(unrewritable)?;
    // Created after planning, so a plan the planner refuses outright leaves no
    // empty destination folder behind. The pre-existing
    // empty-folder-after-rollback case is unchanged and tracked separately.
    create_destination_dirs(vault_root, &target_path, "destination", &asset_moves)?;
    let mutation = execute_note_mutation(
        vault_root,
        entry,
        &target_path,
        expected_content_hash,
        &asset_moves,
        merge_rewrites(backlink_rewrites, asset_rewrites),
        &mut after_phase,
    )?;
    let moved_assets = asset_moves.len();
    // The moved note's own self-link rewrite lands on its destination path,
    // which this operation already reports as its subject. `rewritten_notes`
    // counts the *other* notes, so counting it would report the same write
    // twice, and `affected_paths` would name the destination twice (#254).
    let mut affected_paths = mutation.rewritten;
    let rewrote_its_own_body = affected_paths.contains(&target_path);
    let rewritten_notes = affected_paths.len() - usize::from(rewrote_its_own_body);
    affected_paths.push(entry.path.clone());
    if !rewrote_its_own_body {
        affected_paths.push(target_path.clone());
    }
    for asset in &asset_moves {
        affected_paths.push(asset.source.clone());
        affected_paths.push(asset.destination.clone());
    }

    Ok(WriteOutcome {
        slug: Some(slug),
        relative_path: Some(target_without_ext),
        content_hash: Some(content_hash(&mutation.moved_content)),
        quality_warnings: Vec::new(),
        rewritten_notes,
        moved_assets,
        trashed_path: None,
        affected_paths,
    })
}

pub fn archive_note(
    vault_root: &Path,
    index: &VaultIndex,
    entry: &NoteEntry,
    archive_prefix: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    let archive_folder = archive_prefix.trim().trim_matches('/');
    if archive_folder.is_empty() {
        return Err(WriteError::InvalidInput(
            "archive prefix cannot be empty".to_string(),
        ));
    }
    let file_name = entry
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or(&entry.relative_path);
    let target_relative_path = format!("{archive_folder}/{file_name}");
    if target_relative_path == entry.relative_path {
        return Err(WriteError::Conflict(format!(
            "Note is already archived: {}",
            entry.relative_path
        )));
    }
    // Archiving a demoted note into a default-surface archive folder promotes it:
    // it becomes visible on every default surface. Layer resolution (which the
    // index applies before any archived flag) runs on the *destination* path, so
    // compare source and destination layers and warn the operator when the move
    // reveals a previously-hidden note. `entry.relative_path` and
    // `target_relative_path` are both extension-free, matching what `layer_for`
    // expects.
    if let Some(from_layer) = entry.layer.as_deref()
        && index.layers.layer_for(&target_relative_path).is_none()
    {
        tracing::warn!(
            note = %entry.relative_path,
            from_layer,
            "Archiving promotes a demoted note to the default surface"
        );
    }
    move_or_rename_note(
        vault_root,
        index,
        entry,
        &target_relative_path,
        expected_content_hash,
    )
}

pub fn delete_note(
    vault_root: &Path,
    index: &VaultIndex,
    entry: &NoteEntry,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    delete_note_with_hook(vault_root, index, entry, expected_content_hash, |_| Ok(()))
}

fn delete_note_with_hook(
    vault_root: &Path,
    index: &VaultIndex,
    entry: &NoteEntry,
    expected_content_hash: &str,
    mut after_phase: impl FnMut(MutationPhase) -> Result<(), WriteError>,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let trash_relative = unique_trash_relative_path(vault_root, &entry.relative_path)?;
    let trash_path = vault_root.join(format!("{trash_relative}.md"));

    // No destination: the link is removed from every other note, and the
    // trashed body's link to itself is left as written, since the note is gone
    // from the Vault and the link is moot in the trash (#254).
    let mut unrewritable = Vec::new();
    let backlink_rewrites = backlink_rewrite_plan(index, &entry.slug, None, &mut unrewritable);
    let (asset_moves, asset_rewrites) = asset_move_plan(
        vault_root,
        index,
        entry,
        &trash_path,
        true,
        &backlink_rewrites,
        &mut unrewritable,
    )?;
    refuse_unrewritable(unrewritable)?;
    create_destination_dirs(vault_root, &trash_path, "trash", &asset_moves)?;
    let mutation = execute_note_mutation(
        vault_root,
        entry,
        &trash_path,
        expected_content_hash,
        &asset_moves,
        merge_rewrites(backlink_rewrites, asset_rewrites),
        &mut after_phase,
    )?;
    let moved_assets = asset_moves.len();
    let rewritten = mutation.rewritten;
    let rewritten_notes = rewritten.len();

    let mut affected_paths = rewritten;
    affected_paths.push(entry.path.clone());
    affected_paths.push(trash_path.clone());
    for asset in &asset_moves {
        affected_paths.push(asset.source.clone());
        affected_paths.push(asset.destination.clone());
    }

    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: None,
        quality_warnings: Vec::new(),
        rewritten_notes,
        moved_assets,
        trashed_path: Some(trash_relative),
        affected_paths,
    })
}

/// Create the folders a planned note mutation lands in, once nothing refused
/// it: the note's own, under `note_label`, and each travelling asset's.
fn create_destination_dirs(
    vault_root: &Path,
    note_path: &Path,
    note_label: &str,
    asset_moves: &[AssetMove],
) -> Result<(), WriteError> {
    create_parent_dir_inside_root(vault_root, note_path, note_label)?;
    for asset in asset_moves {
        create_parent_dir_inside_root(vault_root, &asset.destination, "asset")?;
    }
    Ok(())
}

struct CompletedNoteMutation {
    rewritten: Vec<std::path::PathBuf>,
    moved_content: String,
}

fn execute_note_mutation(
    vault_root: &Path,
    entry: &NoteEntry,
    target_path: &Path,
    expected_content_hash: &str,
    asset_moves: &[AssetMove],
    rewrites: Vec<TextRewrite>,
    after_phase: &mut impl FnMut(MutationPhase) -> Result<(), WriteError>,
) -> Result<CompletedNoteMutation, WriteError> {
    let mut journal = MutationJournal::new(vault_root);

    if let Err(error) = journal.move_note(&entry.path, target_path, expected_content_hash) {
        return Err(journal.rollback(error));
    }
    if let Err(error) = after_phase(MutationPhase::Note) {
        return Err(journal.rollback(error));
    }

    for asset in asset_moves {
        if let Err(error) =
            journal.move_file(MutationPhase::Asset, &asset.source, &asset.destination)
        {
            return Err(journal.rollback(error));
        }
    }
    if !asset_moves.is_empty()
        && let Err(error) = after_phase(MutationPhase::Asset)
    {
        return Err(journal.rollback(error));
    }

    let had_rewrites = !rewrites.is_empty();
    let rewritten = match journal.apply_rewrites(rewrites) {
        Ok(rewritten) => rewritten,
        Err(error) => return Err(journal.rollback(error)),
    };
    if had_rewrites && let Err(error) = after_phase(MutationPhase::Rewrite) {
        return Err(journal.rollback(error));
    }

    let moved_content = match fs::read_to_string(target_path) {
        Ok(content) => content,
        Err(error) => {
            return Err(journal.rollback(WriteError::Io(format!(
                "failed to read moved note '{}': {error}",
                target_path.display()
            ))));
        }
    };

    Ok(CompletedNoteMutation {
        rewritten,
        moved_content,
    })
}

#[cfg(test)]
pub(super) fn move_or_rename_note_with_failure(
    vault_root: &Path,
    index: &VaultIndex,
    entry: &NoteEntry,
    target_relative_path: &str,
    expected_content_hash: &str,
    after_phase: impl FnMut(MutationPhase) -> Result<(), WriteError>,
) -> Result<WriteOutcome, WriteError> {
    move_or_rename_note_with_hook(
        vault_root,
        index,
        entry,
        target_relative_path,
        expected_content_hash,
        after_phase,
    )
}

#[cfg(test)]
pub(super) fn delete_note_with_failure(
    vault_root: &Path,
    index: &VaultIndex,
    entry: &NoteEntry,
    expected_content_hash: &str,
    after_phase: impl FnMut(MutationPhase) -> Result<(), WriteError>,
) -> Result<WriteOutcome, WriteError> {
    delete_note_with_hook(vault_root, index, entry, expected_content_hash, after_phase)
}
