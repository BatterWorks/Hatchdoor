//! The contained-resource policy for one Vault's attachments and embedded
//! assets: which relative paths resolve at all, which file types are servable,
//! what content type each carries, how large a response may get, and the
//! address the HTTP asset route answers on.
//!
//! This is deliberately one home for that policy (ADR-19). It used to live in
//! the HTTP adapter (`handlers/assets.rs`), which the MCP `get_attachment` tool
//! imported as a library; both surfaces now reach it only through
//! [`super::VaultReadCore::contained_asset`], so a path one surface refuses is
//! refused by the other for the same reason. The checks themselves are private
//! to this module; what the read core re-exports to adapters is only
//! [`ResolvedAsset`], [`AssetPathError`], [`AssetReadError`], and
//! [`asset_download_path`], so no consumer can reassemble a different policy
//! from the parts.

use std::io::Read;
use std::path::{Component, Path as FsPath, PathBuf};

/// Asset serving is intentionally bounded until a streaming response primitive
/// is introduced. It keeps direct `<img>` and PDF responses from turning one
/// request into an unbounded in-memory allocation.
const MAX_ASSET_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

/// One attachment, resolved, contained, and described without being read.
/// Every field a caller needs to answer about the file — its size, its content
/// type, and the Vault-relative path it was reached by — is settled here, so an
/// adapter needs nothing from this module but this struct and [`AssetPathError`].
pub(crate) struct ResolvedAsset {
    path: PathBuf,
    /// The path relative to the Vault's canonical root. This is how the browse
    /// surface addresses the file; it is not what an adapter echoes back to a
    /// caller, which keeps naming the attachment the way it asked for it.
    pub(crate) relative_path: String,
    pub(crate) size_bytes: u64,
    pub(crate) content_type: &'static str,
}

impl ResolvedAsset {
    /// The attachment's bytes, under the same bound the HTTP route serves.
    pub(crate) fn read_bytes(&self) -> Result<Vec<u8>, AssetReadError> {
        read_asset_bytes_with_limit(&self.path, MAX_ASSET_RESPONSE_BYTES)
    }
}

/// Resolve one Vault-relative attachment path against an already-gated Vault
/// directory and describe what is there, without reading it.
///
/// Containment is established against the *canonical* root, so a symlink that
/// escapes the Vault is refused however it is spelled. The extension
/// allow-list is the same one the asset index applies (#158), so wikilink
/// resolution can never name a path this refuses.
pub(super) fn describe_asset(
    vault_root: &FsPath,
    raw_path: &str,
) -> Result<ResolvedAsset, AssetPathError> {
    let relative = sanitize_asset_path(raw_path).ok_or(AssetPathError::BadRequest)?;
    if !is_allowed_asset_extension(&relative) {
        return Err(AssetPathError::Forbidden);
    }

    let root = std::fs::canonicalize(vault_root).map_err(|_| AssetPathError::Internal)?;
    let candidate = vault_root.join(relative);
    let path = match std::fs::canonicalize(candidate) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(AssetPathError::NotFound);
        }
        Err(_) => return Err(AssetPathError::Internal),
    };

    if !path.starts_with(&root) {
        return Err(AssetPathError::Forbidden);
    }
    if !path.is_file() {
        return Err(AssetPathError::NotFound);
    }

    // Containment is already established; this only names the contained file
    // the way the browse surface and the download URL address it.
    let relative_path = path
        .strip_prefix(&root)
        .map_err(|_| AssetPathError::Forbidden)?
        .to_string_lossy()
        .replace('\\', "/");

    let size_bytes = std::fs::metadata(&path)
        .map_err(|_| AssetPathError::Internal)?
        .len();
    let content_type = content_type_for_path(&path);
    Ok(ResolvedAsset {
        path,
        relative_path,
        size_bytes,
        content_type,
    })
}

/// The path component of the Vault asset route's own URL for one attachment,
/// percent-encoded a segment at a time. It lives with the containment policy so
/// a caller building a link to an attachment (the MCP `get_attachment` tool)
/// cannot drift from the path the route really serves, and mirrors the
/// frontend's encoding of the same route
/// (`frontend/src/components/note-page/wikilinks.ts`).
pub(crate) fn asset_download_path(vault_id: &str, relative_path: &str) -> String {
    let encoded = relative_path
        .split('/')
        .map(percent_encode_segment)
        .collect::<Vec<_>>()
        .join("/");
    format!("/api/v1/vaults/{vault_id}/assets/{encoded}")
}

/// Percent-encode one URL path segment: everything outside RFC 3986's
/// unreserved set. Deliberately not shared with `handlers/downloads.rs`'s
/// `percent_encode_filename`, which escapes the same byte set for a different
/// contract (a `filename*=UTF-8''` header parameter) and belongs to that
/// adapter.
fn percent_encode_segment(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        let is_safe = matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~'
        );
        if is_safe {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}

fn read_asset_bytes_with_limit(path: &FsPath, maximum: u64) -> Result<Vec<u8>, AssetReadError> {
    let file = std::fs::File::open(path).map_err(|error| {
        AssetReadError::Io(format!(
            "failed opening asset '{}': {error}",
            path.display()
        ))
    })?;
    let mut bytes = Vec::new();
    file.take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            AssetReadError::Io(format!(
                "failed reading asset '{}': {error}",
                path.display()
            ))
        })?;
    if bytes.len() as u64 > maximum {
        return Err(AssetReadError::TooLarge);
    }
    Ok(bytes)
}

fn sanitize_asset_path(raw_path: &str) -> Option<PathBuf> {
    let mut sanitized = PathBuf::new();
    let trimmed = raw_path.trim();
    if trimmed.is_empty() || FsPath::new(trimmed).is_absolute() {
        return None;
    }

    for component in FsPath::new(trimmed).components() {
        match component {
            Component::Normal(segment) => sanitized.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir => return None,
            _ => return None,
        }
    }

    if sanitized.as_os_str().is_empty() {
        return None;
    }

    Some(sanitized)
}

fn is_allowed_asset_extension(path: &FsPath) -> bool {
    // Shared with the asset index (#158) so wikilink resolution can never name
    // a path the asset route then refuses.
    crate::vault::is_servable_asset(path)
}

fn content_type_for_path(path: &FsPath) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("avif") => "image/avif",
        Some("bmp") => "image/bmp",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Why one contained-resource request could not be answered. Each variant has
/// exactly one stable `code` and one message, whichever surface reports it; the
/// HTTP status that goes with it belongs to the HTTP adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssetPathError {
    BadRequest,
    Forbidden,
    NotFound,
    Internal,
    TooLarge,
}

impl AssetPathError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::BadRequest => "invalid_asset_path",
            Self::Forbidden => "asset_access_denied",
            Self::NotFound => "asset_not_found",
            Self::TooLarge => "asset_too_large",
            Self::Internal => "internal_error",
        }
    }

    pub(crate) fn message(self, requested_path: &str) -> String {
        match self {
            Self::BadRequest => format!("Invalid asset path: {requested_path}"),
            Self::Forbidden => format!("Asset access denied: {requested_path}"),
            Self::NotFound => format!("Asset not found: {requested_path}"),
            Self::TooLarge => format!("Asset is too large to serve: {requested_path}"),
            Self::Internal => "Asset resolution failed".to_string(),
        }
    }
}

pub(crate) enum AssetReadError {
    TooLarge,
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn sanitize_asset_path_rejects_invalid_paths() {
        assert!(sanitize_asset_path("").is_none());
        assert!(sanitize_asset_path("../secrets.png").is_none());
        assert!(sanitize_asset_path("/abs/path.png").is_none());
        assert!(sanitize_asset_path("folder/../../escape.png").is_none());
    }

    #[test]
    fn sanitize_asset_path_normalizes_valid_path() {
        let path = sanitize_asset_path("./images/diagram.png").expect("valid path");
        assert_eq!(path, PathBuf::from("images/diagram.png"));
    }

    #[test]
    fn is_allowed_asset_extension_filters_by_embeddable_asset_types() {
        assert!(is_allowed_asset_extension(FsPath::new("diagram.png")));
        assert!(is_allowed_asset_extension(FsPath::new("photo.JPEG")));
        assert!(is_allowed_asset_extension(FsPath::new("manual.PDF")));
        assert!(!is_allowed_asset_extension(FsPath::new("notes.md")));
        assert!(!is_allowed_asset_extension(FsPath::new("noext")));
    }

    #[test]
    fn content_type_for_path_serves_pdf_attachments_inline() {
        assert_eq!(
            content_type_for_path(FsPath::new("Attachments/manual.pdf")),
            "application/pdf"
        );
    }

    #[test]
    fn read_asset_bytes_rejects_files_past_the_response_cap() {
        let tmp = TempDir::new().expect("temp dir");
        let path = tmp.path().join("large.png");
        let file = std::fs::File::create(&path).expect("file");
        file.set_len(5).expect("set sparse length");

        assert!(matches!(
            read_asset_bytes_with_limit(&path, 4),
            Err(AssetReadError::TooLarge)
        ));
    }

    #[test]
    fn describe_asset_returns_a_contained_file_and_its_vault_relative_path() {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        let notes_dir = vault_root.join("Notes");
        fs::create_dir_all(&notes_dir).expect("create dir");
        let image_path = notes_dir.join("diagram.png");
        fs::write(&image_path, b"png").expect("write image");

        let resolved =
            describe_asset(&vault_root, "Notes/diagram.png").expect("path should resolve");

        assert_eq!(
            resolved.path,
            std::fs::canonicalize(image_path).expect("canonical image path")
        );
        assert_eq!(resolved.relative_path, "Notes/diagram.png");
        assert_eq!(resolved.content_type, "image/png");
        assert_eq!(resolved.size_bytes, 3);
    }

    #[test]
    fn describe_asset_serves_assets_under_noise_paths() {
        // Noise patterns must never gate contained-asset serving on an ordinary
        // instance: an image embedded in a demoted or otherwise noise-matched
        // folder still renders. A user HATCHDOOR_EXCLUDE glob silently breaking
        // an embedded image would be a nasty surprise, so containment
        // deliberately ignores exclusion; the demo surface applies its own
        // catalogue check on top (`VaultReadCore::asset_on_surface`).
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        let noise_dir = vault_root.join(".trash");
        fs::create_dir_all(&noise_dir).expect("create noise dir");
        let image_path = noise_dir.join("diagram.png");
        fs::write(&image_path, b"png").expect("write image");

        let resolved = describe_asset(&vault_root, ".trash/diagram.png")
            .expect("a noise-path asset must still resolve and serve");
        assert_eq!(
            resolved.path,
            std::fs::canonicalize(image_path).expect("canonical image path")
        );
    }

    #[test]
    fn describe_asset_blocks_traversal_and_non_images() {
        let tmp = TempDir::new().expect("temp dir");
        let vault_root = tmp.path().join("vault");
        fs::create_dir_all(&vault_root).expect("create dir");
        fs::write(vault_root.join("secret.txt"), b"secret").expect("write text");
        fs::write(vault_root.join("demo.mp4"), b"video").expect("write video");

        assert_eq!(
            describe_asset(&vault_root, "../outside.png").err(),
            Some(AssetPathError::BadRequest)
        );
        assert_eq!(
            describe_asset(&vault_root, "secret.txt").err(),
            Some(AssetPathError::Forbidden)
        );
        // #247 widened what the Vault will organise, deliberately not what it
        // will serve: a video the attachment tools now move and list is still
        // refused by this route and by the `get_attachment` tool above it.
        assert_eq!(
            describe_asset(&vault_root, "demo.mp4").err(),
            Some(AssetPathError::Forbidden)
        );
        assert_eq!(
            describe_asset(&vault_root, "missing.png").err(),
            Some(AssetPathError::NotFound)
        );
    }

    #[test]
    fn asset_download_path_percent_encodes_each_segment_but_not_the_separators() {
        assert_eq!(
            asset_download_path("vault-1", "My Folder/a b.png"),
            "/api/v1/vaults/vault-1/assets/My%20Folder/a%20b.png"
        );
    }
}
