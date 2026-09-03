use std::fs;
use std::path::{Path, PathBuf};

use crate::cache::parse::content_hash;
use crate::vault::paths::{slugify, strip_md_extension};
use crate::vault::types::{NoteEntry, VaultIndex};

use super::assets::asset_move_plan;
use super::fs_ops::{
    MutationJournal, atomic_write, atomic_write_if_unchanged, ensure_content_hash,
};
use super::paths::{
    create_parent_dir_inside_root, normalize_note_relative_path, resolve_new_note_path,
    unique_trash_relative_path,
};
use super::rewrites::{backlink_rewrite_plan, merge_rewrites, parse_fence_marker};
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

struct PreparedNoteContent {
    content: String,
    warnings: Vec<String>,
}

fn prepare_note_content(content: &str) -> Result<PreparedNoteContent, WriteError> {
    if content.contains('\0') {
        return Err(WriteError::InvalidInput(
            "note content cannot contain NUL bytes".to_string(),
        ));
    }

    let mut warnings = Vec::new();
    let mut normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    if normalized != content {
        warnings.push("normalized CRLF/CR line endings to LF".to_string());
    }
    if !normalized.is_empty() && !normalized.ends_with('\n') {
        normalized.push('\n');
        warnings.push("added final newline".to_string());
    }
    warnings.extend(frontmatter_warnings(&normalized));

    Ok(PreparedNoteContent {
        content: normalized,
        warnings,
    })
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
    if path.exists() && !overwrite {
        return Err(WriteError::Conflict(format!(
            "Note already exists: {}",
            normalize_note_relative_path(relative_path)?
        )));
    }

    create_parent_dir_inside_root(vault_root, &path, "note")?;

    let prepared = prepare_note_content(content)?;
    atomic_write(&path, &prepared.content)?;
    let normalized = normalize_note_relative_path(relative_path)?;
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
/// mention survive untouched. A `null` value deletes the key. The Markdown
/// body outside the leading frontmatter block is preserved byte-for-byte; a
/// note with no block gets one created. Deleting a note's last frontmatter
/// key removes the now-empty block entirely.
///
/// Only the body is promised byte-stable: the block itself is re-serialized
/// through `serde_json::Map`, so surviving keys come back deterministically
/// sorted and untouched values are re-formatted to serde_yaml_ng's canonical
/// style even when they themselves were not mentioned. Reuses the canonical
/// cache-layer frontmatter parsing (`cache/parse.rs`) so reads and writes
/// agree on what the block is.
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
    // primitives: a merge parses the block through serde_yaml_ng, which
    // silently collapses duplicate keys (last one wins), so that loss is
    // reported as a warning rather than dropped silently.
    let warnings = frontmatter_warnings(&content);
    let mut merged = match &content[span.map(|(start, end)| start..end).unwrap_or(0..0)] {
        "" => serde_json::Map::new(),
        frontmatter => match serde_yaml_ng::from_str::<serde_json::Value>(frontmatter) {
            Ok(serde_json::Value::Object(properties)) => properties,
            Ok(_) | Err(_) => {
                return Err(WriteError::InvalidInput(format!(
                    "note '{}' has invalid YAML frontmatter; fix it directly in the vault before updating it through the API",
                    entry.relative_path
                )));
            }
        },
    };
    for (key, value) in updates {
        if value.is_null() {
            merged.remove(&key);
        } else {
            merged.insert(key, value);
        }
    }

    let updated = if merged.is_empty() {
        if !had_frontmatter {
            return Err(WriteError::InvalidInput(
                "update_frontmatter cannot create an empty frontmatter block; only null values were supplied".to_string(),
            ));
        }
        // Every key was deleted: strip the whole block instead of leaving an
        // empty `---\n---` pair behind. `end + 4` skips the closing "\n---",
        // so the body keeps its exact bytes including any leading newline.
        content[span.map(|(_, end)| end + 4).unwrap_or_default()..].to_string()
    } else {
        let yaml = serde_yaml_ng::to_string(&merged).map_err(|error| {
            WriteError::InvalidInput(format!(
                "updated frontmatter cannot be serialized as YAML: {error}"
            ))
        })?;
        // The canonical parser closes the block at the first `\n---`, so a
        // serialized value whose own lines contain a bare `---` would corrupt
        // every later read of the note. Refuse instead of writing.
        if yaml.split('\n').any(|line| line.trim_end() == "---") {
            return Err(WriteError::InvalidInput(
                "updated frontmatter values may not serialize a line containing only '---'"
                    .to_string(),
            ));
        }
        match span {
            // Rewrite exactly the inner region so the opening/closing markers
            // and everything after them keep their original bytes. The yaml
            // text ends with its own newline, which `content[end..]`'s leading
            // `\n---` replaces.
            Some((start, end)) => {
                format!(
                    "{}{}{}",
                    &content[..start],
                    yaml.trim_end_matches('\n'),
                    &content[end..]
                )
            }
            None => format!("---\n{yaml}---\n{content}"),
        }
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

pub fn append_note(
    entry: &NoteEntry,
    content: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let mut current = fs::read_to_string(&entry.path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read note '{}': {error}",
            entry.relative_path
        ))
    })?;
    if !current.ends_with('\n') {
        current.push('\n');
    }
    current.push_str(content);
    let prepared = prepare_note_content(&current)?;
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
    let updated = if replace_all {
        current.replace(old_string, new_string)
    } else {
        current.replacen(old_string, new_string, 1)
    };
    let prepared = prepare_note_content(&updated)?;
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
    let updated = match mode {
        SectionMode::Replace => splice(&current[..start], content, &current[end..]),
        SectionMode::Before => splice(&current[..start], content, &current[start..]),
        SectionMode::After => splice(&current[..end], content, &current[end..]),
    };
    let prepared = prepare_note_content(&updated)?;
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

fn read_note(entry: &NoteEntry) -> Result<String, WriteError> {
    fs::read_to_string(&entry.path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read note '{}': {error}",
            entry.relative_path
        ))
    })
}

/// All ATX headings in `content` that are not inside a fenced code block, as
/// `(byte offset of the heading line, heading level, trimmed heading line)`.
fn scan_headings(content: &str) -> Vec<(usize, usize, &str)> {
    let mut headings = Vec::new();
    let mut fenced_marker: Option<(u8, usize)> = None;
    let mut offset = 0usize;
    for line in content.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = body.trim_start();
        if let Some((marker, min_len)) = fenced_marker {
            if let Some((close_marker, close_len)) = parse_fence_marker(trimmed)
                && close_marker == marker
                && close_len >= min_len
            {
                fenced_marker = None;
            }
        } else if let Some(marker) = parse_fence_marker(trimmed) {
            fenced_marker = Some(marker);
        } else {
            let level = trimmed.chars().take_while(|ch| *ch == '#').count();
            if (1..=6).contains(&level)
                && trimmed[level..]
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace)
            {
                headings.push((offset, level, trimmed.trim_end()));
            }
        }
        offset += line.len();
    }
    headings
}

/// Byte range `[start, end)` covering the requested section: the heading line
/// through the body that precedes the next same-or-higher heading (or EOF).
fn section_span(
    content: &str,
    requested: &str,
    relative_path: &str,
) -> Result<(usize, usize), WriteError> {
    let headings = scan_headings(content);
    let matched: Vec<usize> = headings
        .iter()
        .enumerate()
        .filter(|(_, (_, _, text))| *text == requested)
        .map(|(idx, _)| idx)
        .collect();
    match matched.as_slice() {
        [] => Err(WriteError::InvalidInput(format!(
            "heading '{requested}' not found in note '{relative_path}'"
        ))),
        [idx] => {
            let (start, level, _) = headings[*idx];
            let end = headings[idx + 1..]
                .iter()
                .find(|(_, candidate_level, _)| *candidate_level <= level)
                .map(|(offset, _, _)| *offset)
                .unwrap_or(content.len());
            Ok((start, end))
        }
        more => Err(WriteError::Conflict(format!(
            "heading '{requested}' is not unique in note '{relative_path}' ({} matches)",
            more.len()
        ))),
    }
}

/// Join `prefix + block + suffix`, guaranteeing newline separation so an
/// inserted block never glues onto adjacent lines.
fn splice(prefix: &str, block: &str, suffix: &str) -> String {
    let mut out = String::with_capacity(prefix.len() + block.len() + suffix.len() + 2);
    out.push_str(prefix);
    if !prefix.is_empty() && !prefix.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(block);
    if !suffix.is_empty() && !block.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(suffix);
    out
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
    let backlink_rewrites = backlink_rewrite_plan(index, &entry.slug, Some(&target_without_ext))?;
    let (asset_moves, asset_rewrites) = asset_move_plan(
        vault_root,
        index,
        entry,
        &target_path,
        false,
        &backlink_rewrites,
    )?;
    // Created after planning, so a plan the planner refuses outright leaves no
    // empty destination folder behind. A plan that carries assets still creates
    // folders while planning them; the pre-existing empty-folder-after-rollback
    // case is unchanged and tracked separately.
    create_parent_dir_inside_root(vault_root, &target_path, "destination")?;
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
    let rewritten = mutation.rewritten;
    let rewritten_notes = rewritten.len();

    let mut affected_paths = rewritten;
    affected_paths.push(entry.path.clone());
    affected_paths.push(target_path.clone());
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

    let backlink_rewrites = backlink_rewrite_plan(index, &entry.slug, None)?;
    let (asset_moves, asset_rewrites) = asset_move_plan(
        vault_root,
        index,
        entry,
        &trash_path,
        true,
        &backlink_rewrites,
    )?;
    create_parent_dir_inside_root(vault_root, &trash_path, "trash")?;
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
