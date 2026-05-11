use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::cache::parse::content_hash;

use super::paths::strip_md_extension;
use super::types::{NoteEntry, VaultIndex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WriteOutcome {
    pub(crate) slug: Option<String>,
    pub(crate) relative_path: Option<String>,
    pub(crate) content_hash: Option<String>,
    pub(crate) rewritten_notes: usize,
    pub(crate) moved_assets: usize,
    pub(crate) trashed_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WriteError {
    Conflict(String),
    InvalidInput(String),
    Io(String),
}

pub(crate) fn create_note(
    vault_root: &Path,
    relative_path: &str,
    content: &str,
    overwrite: bool,
) -> Result<WriteOutcome, WriteError> {
    let path = resolve_new_note_path(vault_root, relative_path)?;
    if path.exists() && !overwrite {
        return Err(WriteError::Conflict(format!(
            "Note already exists: {}",
            normalize_note_relative_path(relative_path)?
        )));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            WriteError::Io(format!(
                "failed to create note directory '{}': {error}",
                parent.display()
            ))
        })?;
        ensure_existing_path_inside_root(vault_root, parent)?;
    }

    atomic_write(&path, content)?;
    let normalized = normalize_note_relative_path(relative_path)?;
    Ok(WriteOutcome {
        slug: None,
        relative_path: Some(strip_md_extension(&normalized).to_string()),
        content_hash: Some(content_hash(content)),
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
    })
}

pub(crate) fn update_note(
    entry: &NoteEntry,
    content: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    atomic_write(&entry.path, content)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(content)),
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
    })
}

pub(crate) fn append_note(
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
    atomic_write(&entry.path, &current)?;
    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: Some(content_hash(&current)),
        rewritten_notes: 0,
        moved_assets: 0,
        trashed_path: None,
    })
}

pub(crate) fn move_or_rename_note(
    vault_root: &Path,
    index: &VaultIndex,
    entry: &NoteEntry,
    target_relative_path: &str,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let target_path = resolve_new_note_path(vault_root, target_relative_path)?;
    if target_path.exists() {
        return Err(WriteError::Conflict(format!(
            "Destination note already exists: {}",
            normalize_note_relative_path(target_relative_path)?
        )));
    }
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            WriteError::Io(format!(
                "failed to create destination directory '{}': {error}",
                parent.display()
            ))
        })?;
        ensure_existing_path_inside_root(vault_root, parent)?;
    }

    let moved_assets = move_referenced_assets(vault_root, &entry.path, &target_path, false)?;
    fs::rename(&entry.path, &target_path).map_err(|error| {
        WriteError::Io(format!(
            "failed to move note '{}' to '{}': {error}",
            entry.path.display(),
            target_path.display()
        ))
    })?;

    let target_without_ext =
        strip_md_extension(&normalize_note_relative_path(target_relative_path)?).to_string();
    let rewritten_notes = rewrite_backlinks(index, &entry.slug, &target_without_ext)?;
    let moved_content = fs::read_to_string(&target_path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read moved note '{}': {error}",
            target_path.display()
        ))
    })?;

    Ok(WriteOutcome {
        slug: None,
        relative_path: Some(target_without_ext),
        content_hash: Some(content_hash(&moved_content)),
        rewritten_notes,
        moved_assets,
        trashed_path: None,
    })
}

pub(crate) fn delete_note(
    vault_root: &Path,
    index: &VaultIndex,
    entry: &NoteEntry,
    expected_content_hash: &str,
) -> Result<WriteOutcome, WriteError> {
    ensure_content_hash(entry, expected_content_hash)?;
    let trash_relative = unique_trash_relative_path(vault_root, &entry.relative_path)?;
    let trash_path = vault_root.join(format!("{trash_relative}.md"));
    if let Some(parent) = trash_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            WriteError::Io(format!(
                "failed to create trash directory '{}': {error}",
                parent.display()
            ))
        })?;
        ensure_existing_path_inside_root(vault_root, parent)?;
    }

    let moved_assets = move_referenced_assets(vault_root, &entry.path, &trash_path, true)?;
    fs::rename(&entry.path, &trash_path).map_err(|error| {
        WriteError::Io(format!(
            "failed to move note '{}' to trash '{}': {error}",
            entry.path.display(),
            trash_path.display()
        ))
    })?;
    let rewritten_notes = rewrite_backlinks(index, &entry.slug, &trash_relative)?;

    Ok(WriteOutcome {
        slug: Some(entry.slug.clone()),
        relative_path: Some(entry.relative_path.clone()),
        content_hash: None,
        rewritten_notes,
        moved_assets,
        trashed_path: Some(trash_relative),
    })
}

pub(crate) fn normalize_note_relative_path(input: &str) -> Result<String, WriteError> {
    let trimmed = input.trim().replace('\\', "/");
    if trimmed.is_empty() {
        return Err(WriteError::InvalidInput(
            "note path cannot be empty".to_string(),
        ));
    }
    let path = Path::new(&trimmed);
    if path.is_absolute() {
        return Err(WriteError::InvalidInput(
            "note path must be vault-relative".to_string(),
        ));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let Some(part) = value.to_str() else {
                    return Err(WriteError::InvalidInput(
                        "note path must be valid UTF-8".to_string(),
                    ));
                };
                if part.is_empty() {
                    return Err(WriteError::InvalidInput(
                        "note path cannot contain empty segments".to_string(),
                    ));
                }
                parts.push(part.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(WriteError::InvalidInput(
                    "note path cannot escape the vault".to_string(),
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err(WriteError::InvalidInput(
            "note path cannot be empty".to_string(),
        ));
    }
    let mut normalized = parts.join("/");
    if !normalized.ends_with(".md") {
        normalized.push_str(".md");
    }
    if normalized == ".md" || normalized.ends_with("/.md") {
        return Err(WriteError::InvalidInput(
            "note filename cannot be empty".to_string(),
        ));
    }
    Ok(normalized)
}

fn resolve_new_note_path(vault_root: &Path, relative_path: &str) -> Result<PathBuf, WriteError> {
    let root = canonical_root(vault_root)?;
    let normalized = normalize_note_relative_path(relative_path)?;
    let path = vault_root.join(&normalized);
    if let Some(parent) = path.parent()
        && parent.exists()
    {
        ensure_existing_path_inside_root(&root, parent)?;
    }
    Ok(path)
}

fn canonical_root(root: &Path) -> Result<PathBuf, WriteError> {
    fs::canonicalize(root).map_err(|error| {
        WriteError::Io(format!(
            "failed to canonicalize vault root '{}': {error}",
            root.display()
        ))
    })
}

fn ensure_existing_path_inside_root(root: &Path, path: &Path) -> Result<(), WriteError> {
    let root = canonical_root(root)?;
    let path = fs::canonicalize(path).map_err(|error| {
        WriteError::Io(format!(
            "failed to canonicalize path '{}': {error}",
            path.display()
        ))
    })?;
    if !path.starts_with(&root) {
        return Err(WriteError::InvalidInput(
            "path cannot escape the vault".to_string(),
        ));
    }
    Ok(())
}

fn ensure_content_hash(entry: &NoteEntry, expected: &str) -> Result<(), WriteError> {
    let expected = expected.trim();
    if expected.is_empty() {
        return Err(WriteError::InvalidInput(
            "expected_content_hash cannot be empty".to_string(),
        ));
    }
    let content = fs::read_to_string(&entry.path).map_err(|error| {
        WriteError::Io(format!(
            "failed to read note '{}': {error}",
            entry.relative_path
        ))
    })?;
    let actual = content_hash(&content);
    if actual != expected {
        return Err(WriteError::Conflict(format!(
            "note changed since it was read: expected {expected}, found {actual}"
        )));
    }
    Ok(())
}

fn atomic_write(path: &Path, content: &str) -> Result<(), WriteError> {
    let tmp = path.with_extension("md.hatchdoor-tmp");
    fs::write(&tmp, content).map_err(|error| {
        WriteError::Io(format!(
            "failed to write temporary note '{}': {error}",
            tmp.display()
        ))
    })?;
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        WriteError::Io(format!(
            "failed to replace note '{}': {error}",
            path.display()
        ))
    })
}

fn rewrite_backlinks(
    index: &VaultIndex,
    moved_slug: &str,
    new_target: &str,
) -> Result<usize, WriteError> {
    let mut changed = 0usize;
    for entry in index.ordered_entries() {
        if entry.slug == moved_slug {
            continue;
        }
        let content = fs::read_to_string(&entry.path).map_err(|error| {
            WriteError::Io(format!(
                "failed to read note '{}' for backlink rewrite: {error}",
                entry.relative_path
            ))
        })?;
        let rewritten = rewrite_wikilinks(
            &content,
            |target| {
                index
                    .resolve_wikilink(target)
                    .is_some_and(|candidate| candidate.slug == moved_slug)
            },
            new_target,
        );
        if rewritten != content {
            atomic_write(&entry.path, &rewritten)?;
            changed += 1;
        }
    }
    Ok(changed)
}

fn rewrite_wikilinks<F>(content: &str, should_rewrite: F, new_target: &str) -> String
where
    F: Fn(&str) -> bool,
{
    let mut out = String::with_capacity(content.len());
    let mut fenced_marker: Option<(u8, usize)> = None;
    for line in content.split_inclusive('\n') {
        let (line_body, line_ending) = line
            .strip_suffix('\n')
            .map(|body| (body, "\n"))
            .unwrap_or((line, ""));
        let trimmed = line_body.trim_start();
        if let Some((marker, min_len)) = fenced_marker {
            if let Some((close_marker, close_len)) = parse_fence_marker(trimmed)
                && close_marker == marker
                && close_len >= min_len
            {
                fenced_marker = None;
            }
            out.push_str(line_body);
            out.push_str(line_ending);
            continue;
        }
        if let Some(marker) = parse_fence_marker(trimmed) {
            fenced_marker = Some(marker);
            out.push_str(line_body);
            out.push_str(line_ending);
            continue;
        }
        out.push_str(&rewrite_wikilinks_in_line(
            line_body,
            &should_rewrite,
            new_target,
        ));
        out.push_str(line_ending);
    }
    out
}

fn rewrite_wikilinks_in_line<F>(line: &str, should_rewrite: &F, new_target: &str) -> String
where
    F: Fn(&str) -> bool,
{
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut idx = 0usize;
    let mut inline_marker_len = 0usize;
    while idx < chars.len() {
        if chars[idx] == '`' {
            let mut marker_len = 1usize;
            while idx + marker_len < chars.len() && chars[idx + marker_len] == '`' {
                marker_len += 1;
            }
            for _ in 0..marker_len {
                out.push('`');
            }
            if inline_marker_len == 0 {
                inline_marker_len = marker_len;
            } else if marker_len == inline_marker_len {
                inline_marker_len = 0;
            }
            idx += marker_len;
            continue;
        }
        if inline_marker_len == 0
            && idx + 1 < chars.len()
            && chars[idx] == '['
            && chars[idx + 1] == '['
        {
            let mut end = idx + 2;
            while end + 1 < chars.len() {
                if chars[end] == ']' && chars[end + 1] == ']' {
                    break;
                }
                end += 1;
            }
            if end + 1 < chars.len() {
                let body: String = chars[idx + 2..end].iter().collect();
                out.push_str("[[");
                out.push_str(&rewrite_wikilink_body(&body, should_rewrite, new_target));
                out.push_str("]]");
                idx = end + 2;
                continue;
            }
        }
        out.push(chars[idx]);
        idx += 1;
    }
    out
}

fn rewrite_wikilink_body<F>(body: &str, should_rewrite: &F, new_target: &str) -> String
where
    F: Fn(&str) -> bool,
{
    let target_end = body.find(['|', '#', '^']).unwrap_or(body.len());
    let target = body[..target_end].trim();
    if target.is_empty() || !should_rewrite(target) {
        return body.to_string();
    }
    let suffix = &body[target_end..];
    format!("{new_target}{suffix}")
}

fn parse_fence_marker(trimmed_line: &str) -> Option<(u8, usize)> {
    let bytes = trimmed_line.as_bytes();
    let marker = *bytes.first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let mut len = 1usize;
    while len < bytes.len() && bytes[len] == marker {
        len += 1;
    }
    if len >= 3 { Some((marker, len)) } else { None }
}

fn move_referenced_assets(
    vault_root: &Path,
    source_note: &Path,
    destination_note: &Path,
    allow_trash_collision: bool,
) -> Result<usize, WriteError> {
    let content = fs::read_to_string(source_note).map_err(|error| {
        WriteError::Io(format!(
            "failed to read note '{}' for asset moves: {error}",
            source_note.display()
        ))
    })?;
    let source_dir = source_note.parent().unwrap_or(vault_root);
    let destination_dir = destination_note.parent().unwrap_or(vault_root);
    let mut moved = 0usize;
    let mut seen = HashSet::new();
    for relative_asset in referenced_assets(&content) {
        if !seen.insert(relative_asset.clone()) {
            continue;
        }
        let source_asset = source_dir.join(&relative_asset);
        if !source_asset.exists() || !source_asset.is_file() {
            continue;
        }
        ensure_existing_path_inside_root(vault_root, &source_asset)?;
        let destination_asset = destination_dir.join(&relative_asset);
        if let Some(parent) = destination_asset.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                WriteError::Io(format!(
                    "failed to create asset directory '{}': {error}",
                    parent.display()
                ))
            })?;
            ensure_existing_path_inside_root(vault_root, parent)?;
        }
        if destination_asset.exists() && !allow_trash_collision {
            return Err(WriteError::Conflict(format!(
                "Destination asset already exists: {}",
                destination_asset.display()
            )));
        }
        fs::rename(&source_asset, &destination_asset).map_err(|error| {
            WriteError::Io(format!(
                "failed to move asset '{}' to '{}': {error}",
                source_asset.display(),
                destination_asset.display()
            ))
        })?;
        moved += 1;
    }
    Ok(moved)
}

fn referenced_assets(content: &str) -> Vec<PathBuf> {
    let mut assets = Vec::new();
    for line in content.lines() {
        extract_markdown_assets(line, &mut assets);
        extract_wiki_assets(line, &mut assets);
    }
    assets
}

fn extract_markdown_assets(line: &str, assets: &mut Vec<PathBuf>) {
    let mut rest = line;
    while let Some(start) = rest.find("](") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find(')') else {
            break;
        };
        let target = rest[..end].split_whitespace().next().unwrap_or("").trim();
        if let Some(asset) = asset_path_from_target(target) {
            assets.push(asset);
        }
        rest = &rest[end + 1..];
    }
}

fn extract_wiki_assets(line: &str, assets: &mut Vec<PathBuf>) {
    let mut rest = line;
    while let Some(start) = rest.find("[[") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("]]") else {
            break;
        };
        let target = rest[..end].split('|').next().unwrap_or("").trim();
        if let Some(asset) = asset_path_from_target(target) {
            assets.push(asset);
        }
        rest = &rest[end + 2..];
    }
}

fn asset_path_from_target(target: &str) -> Option<PathBuf> {
    let target = target.trim();
    if target.is_empty()
        || target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with('#')
        || Path::new(target).is_absolute()
    {
        return None;
    }
    let path = Path::new(target);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif" | "bmp" | "pdf"
    ) {
        Some(path.to_path_buf())
    } else {
        None
    }
}

fn unique_trash_relative_path(
    vault_root: &Path,
    relative_path: &str,
) -> Result<String, WriteError> {
    let base = format!(".hatchdoor-trash/{relative_path}");
    let mut candidate = base.clone();
    let mut suffix = 2usize;
    while vault_root.join(format!("{candidate}.md")).exists() {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    Ok(candidate)
}

impl From<io::Error> for WriteError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn build(root: &Path) -> VaultIndex {
        VaultIndex::build(root).expect("build index")
    }

    #[test]
    fn create_note_rejects_traversal_and_writes_markdown() {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();
        assert!(matches!(
            create_note(root, "../Escape.md", "no", false),
            Err(WriteError::InvalidInput(_))
        ));
        create_note(root, "Projects/New", "# New", false).expect("create");
        assert_eq!(
            fs::read_to_string(root.join("Projects/New.md")).expect("read"),
            "# New"
        );
    }

    #[test]
    fn update_note_requires_matching_hash() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("Home.md");
        fs::write(&path, "old").expect("write");
        let index = build(tmp.path());
        let entry = index.find_by_slug("home").expect("home");
        assert!(matches!(
            update_note(entry, "new", "fnv1a64:deadbeef"),
            Err(WriteError::Conflict(_))
        ));
        update_note(entry, "new", &content_hash("old")).expect("update");
        assert_eq!(fs::read_to_string(path).expect("read"), "new");
    }

    #[test]
    fn move_note_rewrites_backlinks_and_moves_referenced_assets() {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();
        fs::create_dir_all(root.join("Notes")).expect("mkdir");
        fs::write(root.join("Notes/Target.md"), "body\n![](image.png)").expect("target");
        fs::write(root.join("Notes/image.png"), "png").expect("asset");
        fs::write(
            root.join("Backlink.md"),
            "[[Target|Alias]] and `[[Target]]`\n```\n[[Target]]\n```",
        )
        .expect("backlink");
        let index = build(root);
        let entry = index.find_by_slug("target").expect("target");

        let outcome = move_or_rename_note(
            root,
            &index,
            entry,
            "Archive/Renamed.md",
            &content_hash("body\n![](image.png)"),
        )
        .expect("move");

        assert_eq!(outcome.rewritten_notes, 1);
        assert_eq!(outcome.moved_assets, 1);
        assert!(root.join("Archive/Renamed.md").exists());
        assert!(root.join("Archive/image.png").exists());
        let backlink = fs::read_to_string(root.join("Backlink.md")).expect("read");
        assert!(backlink.contains("[[Archive/Renamed|Alias]]"));
        assert!(backlink.contains("`[[Target]]`"));
        assert!(backlink.contains("```\n[[Target]]\n```"));
    }

    #[test]
    fn delete_note_moves_note_and_assets_to_trash() {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();
        fs::write(root.join("Target.md"), "body ![](asset.pdf)").expect("target");
        fs::write(root.join("asset.pdf"), "pdf").expect("asset");
        fs::write(root.join("Backlink.md"), "[[Target]]").expect("backlink");
        let index = build(root);
        let entry = index.find_by_slug("target").expect("target");

        let outcome =
            delete_note(root, &index, entry, &content_hash("body ![](asset.pdf)")).expect("delete");

        let trash = outcome.trashed_path.expect("trash path");
        assert!(!root.join("Target.md").exists());
        assert!(root.join(format!("{trash}.md")).exists());
        assert!(root.join(".hatchdoor-trash/asset.pdf").exists());
        assert!(
            fs::read_to_string(root.join("Backlink.md"))
                .expect("backlink")
                .contains("[[.hatchdoor-trash/Target]]")
        );
    }
}
