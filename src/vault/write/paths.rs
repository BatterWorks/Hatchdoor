use std::fs;
use std::path::{Component, Path, PathBuf};

use super::types::WriteError;

pub(super) fn normalize_note_relative_path(input: &str) -> Result<String, WriteError> {
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

pub fn allowed_attachment_extensions() -> &'static [&'static str] {
    &["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "pdf"]
}

pub(super) fn normalize_attachment_relative_path(input: &str) -> Result<String, WriteError> {
    let trimmed = input.trim().replace('\\', "/");
    if trimmed.is_empty() {
        return Err(WriteError::InvalidInput(
            "attachment path cannot be empty".to_string(),
        ));
    }
    let path = Path::new(&trimmed);
    if path.is_absolute() {
        return Err(WriteError::InvalidInput(
            "attachment path must be vault-relative".to_string(),
        ));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let Some(part) = value.to_str() else {
                    return Err(WriteError::InvalidInput(
                        "attachment path must be valid UTF-8".to_string(),
                    ));
                };
                if part.is_empty() {
                    return Err(WriteError::InvalidInput(
                        "attachment path cannot contain empty segments".to_string(),
                    ));
                }
                parts.push(part.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(WriteError::InvalidInput(
                    "attachment path cannot escape the vault".to_string(),
                ));
            }
        }
    }
    let normalized = parts.join("/");
    if normalized.is_empty() {
        return Err(WriteError::InvalidInput(
            "attachment path cannot be empty".to_string(),
        ));
    }
    Ok(normalized)
}

pub(super) fn normalize_staged_filename(input: &str) -> Result<String, WriteError> {
    let trimmed = input.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || Path::new(trimmed).components().count() != 1
    {
        return Err(WriteError::InvalidInput(
            "staged_filename must be a filename, not a path".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

pub(super) fn resolve_new_note_path(
    vault_root: &Path,
    relative_path: &str,
) -> Result<PathBuf, WriteError> {
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

pub(super) fn resolve_new_attachment_path(
    vault_root: &Path,
    relative_path: &str,
) -> Result<PathBuf, WriteError> {
    let root = canonical_root(vault_root)?;
    let normalized = normalize_attachment_relative_path(relative_path)?;
    let path = vault_root.join(normalized);
    if let Some(parent) = path.parent()
        && parent.exists()
    {
        ensure_existing_path_inside_root(&root, parent)?;
    }
    Ok(path)
}

pub(super) fn resolve_existing_attachment_path(
    vault_root: &Path,
    relative_path: &str,
) -> Result<PathBuf, WriteError> {
    let path = resolve_new_attachment_path(vault_root, relative_path)?;
    if !path.exists() || !path.is_file() {
        return Err(WriteError::InvalidInput(format!(
            "Attachment not found: {}",
            normalize_attachment_relative_path(relative_path)?
        )));
    }
    ensure_existing_path_inside_root(vault_root, &path)?;
    Ok(path)
}

/// Whether a path names a note. Compared case-insensitively, and shared with
/// the asset-reference parser so "is this a note or an attachment?" has one
/// answer across the write layer (#247).
pub(super) fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

/// The gate on operations over a file the Vault already stores: a move, a
/// rename, a delete. Ingest policy deliberately does not apply here (#247) —
/// the bytes are already inside the Vault, and a `.mp4` screen recording or a
/// `.csv` an analysis note cites was permanently immovable while it did. What
/// the allowlist was incidentally refusing, and must keep refusing, is
/// narrower than an extension list:
///
/// - A note is not an attachment. Moving one through this path would skip
///   backlink rewriting, slug handling and the content-hash check.
/// - A Vault's own repository is not content. `.git/config` carries no
///   extension, so the allowlist covered it by accident; relocating one out of
///   `.git` breaks a managed-Git Vault. Noise-excluded folders such as
///   `.obsidian/` lost the same accidental cover, and get it back one level
///   up, in the Vault-qualified core that already owns that policy.
pub(super) fn ensure_movable_attachment_path(
    vault_root: &Path,
    path: &Path,
) -> Result<(), WriteError> {
    if is_markdown_path(path) {
        return Err(WriteError::InvalidInput(format!(
            "'{}' is a note, not an attachment; use the note tools to move, rename or delete it",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
        )));
    }
    ensure_outside_repository(vault_root, path)
}

/// Refuses a path that reaches `.git`, as written and as the filesystem sees
/// it. Both passes are needed: a link named anything at all can lead into the
/// repository, and only canonicalization sees through it. A move's destination
/// does not exist yet, so what gets resolved there is its nearest existing
/// ancestor, which is the part a link could hide in.
fn ensure_outside_repository(vault_root: &Path, path: &Path) -> Result<(), WriteError> {
    let refusal = || {
        WriteError::InvalidInput(
            "'.git' holds this Vault's repository internals, not attachments; \
             attachment tools do not touch it"
                .to_string(),
        )
    };
    if names_repository_internals(vault_root, path) {
        return Err(refusal());
    }
    // The Vault's own root is stripped before the components are read, so a
    // Vault that happens to live under a directory called `.git` is not
    // refused wholesale. That means the canonical pass needs the canonical
    // root to strip; without one there is nothing to compare and the
    // as-written pass above stands alone.
    let Ok(root) = canonical_root(vault_root) else {
        return Ok(());
    };
    match fs::canonicalize(nearest_existing_ancestor(path)) {
        Ok(resolved) if names_repository_internals(&root, &resolved) => Err(refusal()),
        _ => Ok(()),
    }
}

fn names_repository_internals(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .any(|component| {
            matches!(component, Component::Normal(name) if name.eq_ignore_ascii_case(".git"))
        })
}

pub(super) fn ensure_uploadable_attachment_path(path: &Path) -> Result<(), WriteError> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| {
            WriteError::InvalidInput("attachment must have an allowed extension".to_string())
        })?;
    if allowed_attachment_extensions().contains(&extension.as_str()) {
        Ok(())
    } else {
        Err(WriteError::InvalidInput(format!(
            "unsupported attachment extension: {extension}"
        )))
    }
}

pub(super) fn canonical_root(root: &Path) -> Result<PathBuf, WriteError> {
    fs::canonicalize(root).map_err(|error| {
        WriteError::Io(format!(
            "failed to canonicalize vault root '{}': {error}",
            root.display()
        ))
    })
}

pub(super) fn ensure_existing_path_inside_root(root: &Path, path: &Path) -> Result<(), WriteError> {
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

pub(super) fn create_parent_dir_inside_root(
    vault_root: &Path,
    path: &Path,
    label: &str,
) -> Result<(), WriteError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let root = canonical_root(vault_root)?;
    let nearest_existing = nearest_existing_ancestor(parent);
    ensure_existing_path_inside_root(&root, &nearest_existing)?;
    fs::create_dir_all(parent).map_err(|error| {
        WriteError::Io(format!(
            "failed to create {label} directory '{}': {error}",
            parent.display()
        ))
    })?;
    ensure_existing_path_inside_root(&root, parent)
}

fn nearest_existing_ancestor(path: &Path) -> PathBuf {
    let mut current = path;
    while !current.exists() {
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }
    current.to_path_buf()
}

pub(super) fn same_existing_path(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

pub(super) fn relative_link_target(
    vault_root: &Path,
    from_note: &Path,
    target: &Path,
) -> Option<String> {
    let from_dir = from_note.parent().unwrap_or(vault_root);
    let from_relative = from_dir.strip_prefix(vault_root).ok()?;
    let target_relative = target.strip_prefix(vault_root).ok()?;
    let from_parts = path_parts(from_relative)?;
    let target_parts = path_parts(target_relative)?;
    let common_len = from_parts
        .iter()
        .zip(target_parts.iter())
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = Vec::new();
    parts.extend((common_len..from_parts.len()).map(|_| "..".to_string()));
    parts.extend(target_parts[common_len..].iter().cloned());
    if parts.is_empty() {
        target_parts.last().cloned()
    } else {
        Some(parts.join("/"))
    }
}

/// The vault-relative path of a directory that sits inside `vault_root`, as a
/// `/`-joined string; the empty string for the vault root itself.
pub(super) fn vault_relative_dir(vault_root: &Path, dir: &Path) -> Option<String> {
    let relative = dir.strip_prefix(vault_root).ok()?;
    Some(path_parts(relative)?.join("/"))
}

/// Resolve a note-relative asset reference against the folder the note lives
/// in, collapsing `.` and `..` lexically, and return the result as a
/// vault-relative path. `None` when the reference climbs out of the vault root
/// or is not representable as plain UTF-8 components under it.
///
/// The collapsing has to happen here rather than being left to the kernel: the
/// low-level opener walks a path one plain name at a time with `openat`, so any
/// `..` that survives into a planned move is refused as an unsupported
/// component (#225). Resolving lexically also keeps the check honest for a
/// destination that does not exist yet.
pub(super) fn resolve_reference_inside_root(
    vault_root: &Path,
    note_dir: &Path,
    reference: &Path,
) -> Option<String> {
    let mut parts = path_parts(note_dir.strip_prefix(vault_root).ok()?)?;
    for component in reference.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_str()?.to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

fn path_parts(path: &Path) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_str()?.to_string()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts)
}

pub(super) fn vault_relative_file_path(
    vault_root: &Path,
    path: &Path,
) -> Result<Option<String>, WriteError> {
    let root = canonical_root(vault_root)?;
    let path = fs::canonicalize(path).map_err(|error| {
        WriteError::Io(format!(
            "failed to canonicalize attachment '{}': {error}",
            path.display()
        ))
    })?;
    if !path.starts_with(&root) {
        return Ok(None);
    }
    let relative = path.strip_prefix(&root).ok().and_then(|path| {
        let parts = path_parts(path)?;
        Some(parts.join("/"))
    });
    Ok(relative)
}

pub(super) fn is_trashed_path(vault_root: &Path, path: &Path) -> Result<bool, WriteError> {
    Ok(vault_relative_file_path(vault_root, path)?
        .as_deref()
        .is_some_and(|relative| {
            relative == ".hatchdoor-trash" || relative.starts_with(".hatchdoor-trash/")
        }))
}

pub(super) fn unique_trash_attachment_relative_path(
    vault_root: &Path,
    relative_path: &str,
) -> Result<String, WriteError> {
    let normalized = normalize_attachment_relative_path(relative_path)?;
    let base = format!(".hatchdoor-trash/{normalized}");
    let mut candidate = base.clone();
    let mut suffix = 2usize;
    while vault_root.join(&candidate).exists() {
        let path = Path::new(&base);
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("attachment");
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|extension| format!(".{extension}"))
            .unwrap_or_default();
        let parent = path
            .parent()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .map(|parent| format!("{parent}/"))
            .unwrap_or_default();
        candidate = format!("{parent}{stem}-{suffix}{extension}");
        suffix += 1;
    }
    Ok(candidate)
}

pub(super) fn unique_trash_relative_path(
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
