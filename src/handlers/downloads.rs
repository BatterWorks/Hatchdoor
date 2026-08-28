use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path as FsPath, PathBuf};

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use zip::write::SimpleFileOptions;

use crate::vault::Note;

/// Note downloads are assembled in memory so Markdown links can be rewritten
/// into a self-contained archive. Bound every intermediate buffer rather than
/// allowing an arbitrarily large note or asset set to exhaust the process.
const MAX_NOTE_EXPORT_BYTES: usize = 64 * 1024 * 1024;

/// Shared response shape for a built [`NoteExport`], used by the Vault-scoped
/// route in `handlers/vault_content.rs`.
pub(crate) fn download_response(export: NoteExport) -> Response {
    let content_disposition = build_download_content_disposition(&export.filename);

    let mut response = Response::new(export.bytes.into());
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(export.content_type),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition)
            .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=\"note.md\"")),
    );

    response
}

pub(crate) struct NoteExport {
    pub(crate) filename: String,
    pub(crate) content_type: &'static str,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug)]
pub(crate) enum ExportError {
    TooLarge,
    Failed(String),
}

fn download_filename_for_note(note: &Note) -> String {
    let from_path = note
        .relative_path
        .split('/')
        .next_back()
        .unwrap_or(note.title.as_str());
    let base = sanitize_download_filename(from_path);
    if base.ends_with(".md") {
        base
    } else {
        format!("{base}.md")
    }
}

pub(crate) fn build_note_export(
    vault_root: &FsPath,
    note: &Note,
) -> Result<NoteExport, ExportError> {
    let markdown_filename = download_filename_for_note(note);
    let markdown = clean_markdown_export(&note.content);
    if markdown.len() > MAX_NOTE_EXPORT_BYTES {
        return Err(ExportError::TooLarge);
    }
    let assets = export_assets(vault_root, note, &markdown, &markdown_filename);
    if assets.is_empty() {
        return Ok(NoteExport {
            filename: markdown_filename,
            content_type: "text/markdown; charset=utf-8",
            bytes: markdown.into_bytes(),
        });
    }

    let zip_filename = markdown_filename
        .strip_suffix(".md")
        .map(|base| format!("{base}.zip"))
        .unwrap_or_else(|| format!("{markdown_filename}.zip"));
    let markdown = rewrite_export_asset_links(&markdown, &assets);
    let bytes = build_zip_export(&markdown_filename, &markdown, &assets)?;

    Ok(NoteExport {
        filename: zip_filename,
        content_type: "application/zip",
        bytes,
    })
}

fn sanitize_download_filename(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.trim().chars() {
        let allowed = ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '-' | '_' | '.' | '(' | ')');
        if allowed {
            output.push(ch);
        } else if !ch.is_ascii_control() {
            output.push('-');
        }
    }

    let collapsed = output.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_end_matches('.').trim();
    if trimmed.is_empty() {
        "note".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn build_download_content_disposition(filename: &str) -> String {
    let ascii_fallback = filename
        .chars()
        .map(|ch| if ch.is_ascii() { ch } else { '-' })
        .collect::<String>();
    format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        ascii_fallback,
        percent_encode_filename(filename)
    )
}

fn clean_markdown_export(input: &str) -> String {
    let without_frontmatter = strip_frontmatter(input);
    strip_vault_note_links(&without_frontmatter)
}

fn strip_frontmatter(input: &str) -> String {
    let lines = input.lines().collect::<Vec<_>>();
    if lines.len() < 3 || lines.first().is_none_or(|line| line.trim() != "---") {
        return input.to_string();
    }

    let Some(end) = lines
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(idx, line)| (line.trim() == "---").then_some(idx))
    else {
        return input.to_string();
    };

    let header = &lines[1..end];
    if !looks_like_frontmatter_header(header) {
        return input.to_string();
    }

    lines[end + 1..].join("\n")
}

#[derive(Debug, Clone)]
struct ExportAsset {
    original_target: String,
    zip_path: String,
    source_path: PathBuf,
}

fn export_assets(
    vault_root: &FsPath,
    note: &Note,
    markdown: &str,
    markdown_filename: &str,
) -> Vec<ExportAsset> {
    let note_dir = note
        .relative_path
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or("");
    let asset_folder = export_asset_folder(markdown_filename);
    let mut seen_sources = HashSet::new();
    let mut used_names = HashSet::new();
    let mut assets = Vec::new();

    for target in referenced_asset_targets(markdown) {
        let Some(relative_target) = normalize_export_asset_target(&target) else {
            continue;
        };
        let resolved_relative = normalize_relative_path(note_dir, &relative_target);
        if resolved_relative.as_os_str().is_empty() {
            continue;
        }
        let source = vault_root.join(&resolved_relative);
        let Ok(source) = std::fs::canonicalize(&source) else {
            continue;
        };
        let Ok(root) = std::fs::canonicalize(vault_root) else {
            continue;
        };
        if !source.starts_with(&root) || !source.is_file() {
            continue;
        }
        if !seen_sources.insert(source.clone()) {
            continue;
        }
        let zip_filename = unique_asset_filename(&source, &mut used_names);
        let zip_path = format!("{asset_folder}/{zip_filename}");
        assets.push(ExportAsset {
            original_target: target,
            zip_path,
            source_path: source,
        });
    }

    assets
}

fn export_asset_folder(markdown_filename: &str) -> String {
    markdown_filename
        .strip_suffix(".md")
        .unwrap_or(markdown_filename)
        .trim()
        .to_string()
        + "-assets"
}

fn referenced_asset_targets(markdown: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for line in markdown.lines() {
        extract_markdown_asset_targets(line, &mut targets);
        extract_wiki_asset_targets(line, &mut targets);
    }
    targets
}

fn extract_markdown_asset_targets(line: &str, targets: &mut Vec<String>) {
    let mut rest = line;
    while let Some(start) = rest.find("](") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find(')') else {
            break;
        };
        let target = rest[..end].split_whitespace().next().unwrap_or("").trim();
        if is_export_asset_target(target) {
            targets.push(target.to_string());
        }
        rest = &rest[end + 1..];
    }
}

fn extract_wiki_asset_targets(line: &str, targets: &mut Vec<String>) {
    let mut rest = line;
    while let Some(start) = rest.find("![[") {
        rest = &rest[start + 3..];
        let Some(end) = rest.find("]]") else {
            break;
        };
        let target = rest[..end].split('|').next().unwrap_or("").trim();
        if is_export_asset_target(target) {
            targets.push(target.to_string());
        }
        rest = &rest[end + 2..];
    }
}

fn is_export_asset_target(target: &str) -> bool {
    let target = target.trim();
    if target.is_empty()
        || target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("data:")
        || target.starts_with("blob:")
        || target.starts_with("/n/")
        || target.starts_with("/__missing__/")
        || target.starts_with('#')
        || FsPath::new(target).is_absolute()
    {
        return false;
    }
    let path = FsPath::new(target);
    let Some(ext) = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
    else {
        return false;
    };
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif" | "bmp" | "pdf"
    )
}

fn normalize_export_asset_target(target: &str) -> Option<PathBuf> {
    let target = target.split(['?', '#']).next().unwrap_or("").trim();
    if !is_export_asset_target(target) {
        return None;
    }
    Some(FsPath::new(target).to_path_buf())
}

fn normalize_relative_path(base_dir: &str, target: &FsPath) -> PathBuf {
    let mut stack = PathBuf::new();
    for component in FsPath::new(base_dir).components() {
        if let Component::Normal(segment) = component {
            stack.push(segment);
        }
    }
    for component in target.components() {
        match component {
            Component::Normal(segment) => stack.push(segment),
            Component::CurDir => {}
            Component::ParentDir => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack
}

fn unique_asset_filename(source: &FsPath, used: &mut HashSet<String>) -> String {
    let fallback = "asset".to_string();
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .map(sanitize_download_filename)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback);
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    let mut suffix = 1usize;
    loop {
        let name = match (&extension, suffix) {
            (Some(ext), 1) => format!("{stem}.{ext}"),
            (Some(ext), _) => format!("{stem}-{suffix}.{ext}"),
            (None, 1) => stem.clone(),
            (None, _) => format!("{stem}-{suffix}"),
        };
        if used.insert(name.clone()) {
            return name;
        }
        suffix += 1;
    }
}

fn rewrite_export_asset_links(markdown: &str, assets: &[ExportAsset]) -> String {
    let target_map = assets
        .iter()
        .map(|asset| (asset.original_target.as_str(), asset.zip_path.as_str()))
        .collect::<HashMap<_, _>>();
    let mut rewritten = rewrite_markdown_asset_links(markdown, &target_map);
    rewritten = rewrite_wiki_asset_links(&rewritten, &target_map);
    rewritten
}

fn rewrite_markdown_asset_links(markdown: &str, target_map: &HashMap<&str, &str>) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(start) = rest.find("](") {
        out.push_str(&rest[..start + 2]);
        rest = &rest[start + 2..];
        let Some(end) = rest.find(')') else {
            out.push_str(rest);
            return out;
        };
        let body = &rest[..end];
        let target = body.split_whitespace().next().unwrap_or("").trim();
        if let Some(zip_path) = target_map.get(target) {
            out.push_str(zip_path);
            out.push_str(&body[target.len()..]);
        } else {
            out.push_str(body);
        }
        out.push(')');
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn rewrite_wiki_asset_links(markdown: &str, target_map: &HashMap<&str, &str>) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(start) = rest.find("![[") {
        out.push_str(&rest[..start]);
        rest = &rest[start + 3..];
        let Some(end) = rest.find("]]") else {
            out.push_str("![[");
            out.push_str(rest);
            return out;
        };
        let body = &rest[..end];
        let target = body.split('|').next().unwrap_or("").trim();
        let label = body
            .split_once('|')
            .map(|(_, label)| label.trim())
            .filter(|label| !label.is_empty())
            .unwrap_or(target);
        if let Some(zip_path) = target_map.get(target) {
            out.push_str(&format!(
                "![{}]({})",
                escape_markdown_label(label),
                zip_path
            ));
        } else {
            out.push_str("![[");
            out.push_str(body);
            out.push_str("]]");
        }
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    out
}

fn strip_vault_note_links(markdown: &str) -> String {
    let without_wikilinks = strip_note_wikilinks(markdown);
    strip_internal_markdown_links(&without_wikilinks)
}

fn strip_note_wikilinks(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(start) = rest.find("[[") {
        out.push_str(&rest[..start]);
        let is_embed = start > 0 && rest.as_bytes()[start - 1] == b'!';
        rest = &rest[start + 2..];
        let Some(end) = rest.find("]]") else {
            out.push_str("[[");
            out.push_str(rest);
            return out;
        };
        let body = &rest[..end];
        if is_embed {
            out.push_str("[[");
            out.push_str(body);
            out.push_str("]]");
        } else {
            out.push_str(wikilink_label(body));
        }
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    out
}

fn wikilink_label(body: &str) -> &str {
    body.split_once('|')
        .map(|(_, label)| label.trim())
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| body.split(['#', '^']).next().unwrap_or(body).trim())
}

fn strip_internal_markdown_links(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(start) = rest.find("](") {
        let Some(label_start) = rest[..start].rfind('[') else {
            out.push_str(&rest[..start + 2]);
            rest = &rest[start + 2..];
            continue;
        };
        if label_start > 0 && rest.as_bytes()[label_start - 1] == b'!' {
            out.push_str(&rest[..start + 2]);
            rest = &rest[start + 2..];
            continue;
        }
        let label = &rest[label_start + 1..start];
        out.push_str(&rest[..label_start]);
        rest = &rest[start + 2..];
        let Some(end) = rest.find(')') else {
            out.push('[');
            out.push_str(label);
            out.push_str("](");
            out.push_str(rest);
            return out;
        };
        let target = rest[..end].trim();
        if target.starts_with("/n/") || target.starts_with("/__missing__/") {
            out.push_str(label);
        } else {
            out.push('[');
            out.push_str(label);
            out.push_str("](");
            out.push_str(&rest[..end]);
            out.push(')');
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn build_zip_export(
    markdown_filename: &str,
    markdown: &str,
    assets: &[ExportAsset],
) -> Result<Vec<u8>, ExportError> {
    let mut writer = zip::ZipWriter::new(LimitedExportWriter::new(MAX_NOTE_EXPORT_BYTES));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    writer
        .start_file(markdown_filename, options)
        .map_err(|error| {
            ExportError::Failed(format!("failed to add markdown to export zip: {error}"))
        })?;
    writer.write_all(markdown.as_bytes()).map_err(|error| {
        ExportError::Failed(format!("failed to write markdown to export zip: {error}"))
    })?;

    for asset in assets {
        let bytes = read_export_asset(&asset.source_path)?;
        writer
            .start_file(&asset.zip_path, options)
            .map_err(|error| {
                ExportError::Failed(format!("failed to add asset to export zip: {error}"))
            })?;
        writer.write_all(&bytes).map_err(|error| {
            ExportError::Failed(format!(
                "failed to write asset '{}' to export zip: {error}",
                asset.source_path.display()
            ))
        })?;
    }

    writer
        .finish()
        .map(LimitedExportWriter::into_inner)
        .map_err(|error| ExportError::Failed(format!("failed to finish export zip: {error}")))
}

fn read_export_asset(path: &FsPath) -> Result<Vec<u8>, ExportError> {
    let file = std::fs::File::open(path).map_err(|error| {
        ExportError::Failed(format!(
            "failed opening asset '{}' for export: {error}",
            path.display()
        ))
    })?;
    let mut bytes = Vec::new();
    file.take((MAX_NOTE_EXPORT_BYTES as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            ExportError::Failed(format!(
                "failed reading asset '{}' for export: {error}",
                path.display()
            ))
        })?;
    if bytes.len() > MAX_NOTE_EXPORT_BYTES {
        return Err(ExportError::TooLarge);
    }
    Ok(bytes)
}

struct LimitedExportWriter {
    cursor: Cursor<Vec<u8>>,
    maximum: usize,
}

impl LimitedExportWriter {
    fn new(maximum: usize) -> Self {
        Self {
            cursor: Cursor::new(Vec::new()),
            maximum,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.cursor.into_inner()
    }
}

impl Write for LimitedExportWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let next_len = usize::try_from(self.cursor.position())
            .ok()
            .and_then(|position| position.checked_add(buf.len()))
            .ok_or_else(|| {
                std::io::Error::other("note export exceeds the server download size limit")
            })?;
        if next_len > self.maximum {
            return Err(std::io::Error::other(
                "note export exceeds the server download size limit",
            ));
        }
        self.cursor.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for LimitedExportWriter {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.cursor.seek(position)
    }
}

fn escape_markdown_label(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        if matches!(
            ch,
            '\\' | '`'
                | '*'
                | '_'
                | '['
                | ']'
                | '{'
                | '}'
                | '('
                | ')'
                | '#'
                | '+'
                | '.'
                | '!'
                | '|'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn looks_like_frontmatter_header(lines: &[&str]) -> bool {
    if lines.is_empty() {
        return false;
    }

    let mut has_property = false;
    let mut list_allowed = false;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with("- ") {
            if !list_allowed {
                return false;
            }
            has_property = true;
            continue;
        }

        let Some(colon_idx) = trimmed.find(':') else {
            return false;
        };
        if colon_idx == 0 {
            return false;
        }

        has_property = true;
        list_allowed = trimmed.ends_with(':');
    }

    has_property
}

// Also reused by the MCP `get_attachment` read tool to encode each segment
// of an attachment's relative path for the existing `/assets/{*path}` route.
pub(crate) fn percent_encode_filename(input: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::tempdir;

    #[test]
    fn sanitize_download_filename_replaces_unsafe_chars() {
        assert_eq!(
            sanitize_download_filename("  Prox:mox/Note?.md  "),
            "Prox-mox-Note-.md"
        );
        assert_eq!(sanitize_download_filename(""), "note");
    }

    #[test]
    fn download_filename_for_note_adds_md_extension_when_missing() {
        let note = Note {
            title: "README".to_string(),
            slug: "readme".to_string(),
            relative_path: "Docs/README".to_string(),
            content: "# Readme".to_string(),
            content_hash: "fnv1a64:0000000000000000".to_string(),
            layer: None,
            metadata: Default::default(),
        };

        assert_eq!(download_filename_for_note(&note), "README.md");
    }

    #[test]
    fn build_download_content_disposition_includes_utf8_filename() {
        let value = build_download_content_disposition("Homelab Atlas.md");
        assert_eq!(
            value,
            "attachment; filename=\"Homelab Atlas.md\"; filename*=UTF-8''Homelab%20Atlas.md"
        );
    }

    #[test]
    fn clean_markdown_export_strips_leading_frontmatter() {
        let content = "---\ntags: [vault/sort, status/active]\narea: Home\n---\n# Home\n\nBody";

        assert_eq!(clean_markdown_export(content), "# Home\n\nBody");
    }

    #[test]
    fn clean_markdown_export_keeps_markdown_horizontal_rules() {
        let content = "---\nplain markdown\n---\n# Home";

        assert_eq!(clean_markdown_export(content), content);
    }

    #[test]
    fn clean_markdown_export_strips_vault_only_note_links() {
        let content = "See [[Projects/Plan|Plan Home]], [[Topic#Part]], [Internal](/n/topic), [Missing](/__missing__/Topic), and [External](https://example.com).";

        assert_eq!(
            clean_markdown_export(content),
            "See Plan Home, Topic, Internal, Missing, and [External](https://example.com)."
        );
    }

    #[test]
    fn build_note_export_bundles_local_assets_and_rewrites_links() {
        let dir = tempdir().expect("temp dir");
        let vault = dir.path();
        let notes = vault.join("Notes");
        std::fs::create_dir_all(&notes).expect("notes dir");
        std::fs::write(notes.join("diagram.png"), b"png-bytes").expect("asset");
        let note = Note {
            title: "Home".to_string(),
            slug: "home".to_string(),
            relative_path: "Notes/Home".to_string(),
            content: "---\ntags: [vault/sort]\n---\n# Home\n\nSee [[Plan|Plan]].\n\n![[diagram.png|Topology]]\n\n[External](https://example.com)".to_string(),
            content_hash: "fnv1a64:0000000000000000".to_string(),
            layer: None,
            metadata: Default::default(),
        };

        let export = build_note_export(vault, &note).expect("export");

        assert_eq!(export.filename, "Home.zip");
        assert_eq!(export.content_type, "application/zip");

        let reader = Cursor::new(export.bytes);
        let mut archive = zip::ZipArchive::new(reader).expect("zip archive");
        let mut markdown = String::new();
        archive
            .by_name("Home.md")
            .expect("markdown file")
            .read_to_string(&mut markdown)
            .expect("markdown text");
        assert_eq!(
            markdown,
            "# Home\n\nSee Plan.\n\n![Topology](Home-assets/diagram.png)\n\n[External](https://example.com)"
        );

        let mut asset = Vec::new();
        archive
            .by_name("Home-assets/diagram.png")
            .expect("asset file")
            .read_to_end(&mut asset)
            .expect("asset bytes");
        assert_eq!(asset, b"png-bytes");
    }

    #[test]
    fn limited_export_writer_refuses_to_grow_past_its_bound() {
        let mut writer = LimitedExportWriter::new(3);
        writer.write_all(b"abc").expect("fill limit");
        assert!(writer.write_all(b"d").is_err());
        assert_eq!(writer.into_inner(), b"abc");
    }
}
