use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path as FsPath, PathBuf};

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use zip::write::SimpleFileOptions;

use crate::vault::{Note, split_wikilink_asset_body};

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

/// `admit_asset` decides, per contained Vault-relative asset path, whether the
/// export may carry it. A refused asset is treated exactly like a missing one:
/// left out of the archive, its link left as written.
pub(crate) fn build_note_export(
    vault_root: &FsPath,
    note: &Note,
    admit_asset: impl Fn(&str) -> bool,
) -> Result<NoteExport, ExportError> {
    let markdown_filename = download_filename_for_note(note);
    let markdown = clean_markdown_export(&note.content);
    if markdown.len() > MAX_NOTE_EXPORT_BYTES {
        return Err(ExportError::TooLarge);
    }
    let assets = export_assets(
        vault_root,
        note,
        &markdown,
        &markdown_filename,
        &admit_asset,
    );
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
    let bytes = build_zip_export(
        &markdown_filename,
        &markdown,
        &assets,
        MAX_NOTE_EXPORT_BYTES,
    )?;

    Ok(NoteExport {
        filename: zip_filename,
        content_type: "application/zip",
        bytes,
    })
}

/// A note or asset name made safe to hand a browser or an unzip tool, and to
/// write unescaped into the archive's own Markdown links. The name keeps its
/// own script (#647), up to 200 bytes; control and invisible characters are
/// dropped, and any ASCII
/// character other than letters, digits and ` -_.()` becomes `-`, as it always
/// has: `#` or `%` would turn a link inside the zip into an anchor or an
/// escape. Reducing to ASCII is [`build_download_content_disposition`]'s job,
/// for its fallback alone.
const MAX_DOWNLOAD_NAME_BYTES: usize = 200;

fn sanitize_download_filename(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.trim().chars() {
        if ch.is_control() || is_invisible(ch) {
            continue;
        }
        let kept = !ch.is_ascii()
            || ch.is_ascii_alphanumeric()
            || matches!(ch, ' ' | '-' | '_' | '.' | '(' | ')');
        output.push(if kept { ch } else { '-' });
    }

    let mut collapsed = output.split_whitespace().collect::<Vec<_>>().join(" ");
    // Most filesystems cap a name at 255 bytes, and a name that keeps its own
    // script takes up to 4 bytes a letter. Leave room for `-assets`, a `-2`
    // suffix or an extension added after this.
    if collapsed.len() > MAX_DOWNLOAD_NAME_BYTES {
        let mut end = MAX_DOWNLOAD_NAME_BYTES;
        while !collapsed.is_char_boundary(end) {
            end -= 1;
        }
        collapsed.truncate(end);
    }
    let trimmed = collapsed.trim_end_matches('.').trim();
    if trimmed.is_empty() {
        "note".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Invisible characters: the ones that reorder how a name displays, so
/// `gpj.exe` can read as `exe.jpg`, the Unicode line and paragraph separators,
/// zero-width spaces and joiners, the soft hyphen and the byte-order mark. A
/// downloaded file name never needs them.
fn is_invisible(ch: char) -> bool {
    matches!(
        ch,
        '\u{00AD}' | '\u{200B}'..='\u{200D}' | '\u{2028}' | '\u{2029}' | '\u{2060}' | '\u{FEFF}' |
        '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// RFC 6266 `Content-Disposition` for a download. `filename*` (RFC 5987)
/// carries the real UTF-8 name; the quoted `filename=` is the ASCII fallback
/// for clients that predate it. Whatever the caller passes, no control
/// character, quote or backslash reaches the quoted value.
pub(crate) fn build_download_content_disposition(filename: &str) -> String {
    let ascii_fallback = filename
        .chars()
        .map(|ch| {
            if ch.is_ascii() && !ch.is_ascii_control() && !matches!(ch, '"' | '\\') {
                ch
            } else {
                '-'
            }
        })
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
    admit_asset: &impl Fn(&str) -> bool,
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
        // Contained, and named the way the asset route names a contained
        // file: its canonical path under the canonical root.
        let Ok(contained) = source.strip_prefix(&root) else {
            continue;
        };
        if !source.is_file() || !admit_asset(&contained.to_string_lossy().replace('\\', "/")) {
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
        let (target, _) = split_wikilink_asset_body(&rest[..end]);
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
        let (target, suffix) = split_wikilink_asset_body(body);
        // The label comes out of the same split as the target, so the two can
        // never disagree about where the alias pipe is - escaped or not.
        let label = alias_from_suffix(suffix).unwrap_or(target);
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

/// The display text a wikilink suffix carries, if it carries one.
///
/// `suffix` is what [`split_wikilink_asset_body`] returns after the target:
/// the alias pipe and everything past it, with the backslash that escapes the
/// pipe inside a table cell still attached (#252).
fn alias_from_suffix(suffix: &str) -> Option<&str> {
    suffix
        .strip_prefix('\\')
        .unwrap_or(suffix)
        .strip_prefix('|')
        .map(str::trim)
        .filter(|label| !label.is_empty())
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
    maximum: usize,
) -> Result<Vec<u8>, ExportError> {
    let mut writer = zip::ZipWriter::new(LimitedExportWriter::new(maximum));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let failed = |context: &str, error: &dyn std::fmt::Display| {
        ExportError::Failed(format!("{context}: {error}"))
    };

    writer
        .start_file(markdown_filename, options)
        .map_err(|error| failed("failed to add markdown to export zip", &error))?;
    writer
        .write_all(markdown.as_bytes())
        .map_err(|error| failed("failed to write markdown to export zip", &error))?;

    for asset in assets {
        // Stop at the first entry past the bound: what is already written
        // cannot shrink, and reading the next asset would be wasted work.
        // Bytes the compressor still buffers are caught by the final check.
        if writer
            .get_ref()
            .is_some_and(LimitedExportWriter::overflowed)
        {
            return Err(ExportError::TooLarge);
        }
        let bytes = read_export_asset(&asset.source_path, maximum)?;
        writer
            .start_file(&asset.zip_path, options)
            .map_err(|error| failed("failed to add asset to export zip", &error))?;
        writer.write_all(&bytes).map_err(|error| {
            failed(
                &format!(
                    "failed to write asset '{}' to export zip",
                    asset.source_path.display()
                ),
                &error,
            )
        })?;
    }

    writer
        .finish()
        .map_err(|error| failed("failed to finish export zip", &error))?
        .into_inner()
}

fn read_export_asset(path: &FsPath, maximum: usize) -> Result<Vec<u8>, ExportError> {
    let file = std::fs::File::open(path).map_err(|error| {
        ExportError::Failed(format!(
            "failed opening asset '{}' for export: {error}",
            path.display()
        ))
    })?;
    let mut bytes = Vec::new();
    file.take((maximum as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            ExportError::Failed(format!(
                "failed reading asset '{}' for export: {error}",
                path.display()
            ))
        })?;
    if bytes.len() > maximum {
        return Err(ExportError::TooLarge);
    }
    Ok(bytes)
}

/// The archive's bytes, held only up to `maximum`. The bound is the size of
/// the compressed archive, so a note of well-compressing images still fits.
///
/// Past the bound the writer keeps answering as an ordinary seekable file
/// whose bytes beyond `maximum` are dropped, and only records that it
/// overflowed (#647). Refusing the write instead would leave `ZipWriter`
/// unfinished, and its drop would report the failure on stderr; the caller
/// turns the overflow into [`ExportError::TooLarge`] instead.
struct LimitedExportWriter {
    bytes: Vec<u8>,
    position: u64,
    length: u64,
    maximum: usize,
}

impl LimitedExportWriter {
    fn new(maximum: usize) -> Self {
        Self {
            bytes: Vec::new(),
            position: 0,
            length: 0,
            maximum,
        }
    }

    fn overflowed(&self) -> bool {
        self.length > self.maximum as u64
    }

    fn into_inner(self) -> Result<Vec<u8>, ExportError> {
        if self.overflowed() {
            Err(ExportError::TooLarge)
        } else {
            Ok(self.bytes)
        }
    }
}

impl Write for LimitedExportWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let end = self.position.saturating_add(buf.len() as u64);
        let kept_end = end.min(self.maximum as u64);
        if self.position < kept_end {
            // Both bounds are at most `maximum`, so they fit in `usize`.
            let start = self.position as usize;
            let stop = kept_end as usize;
            if self.bytes.len() < stop {
                self.bytes.resize(stop, 0);
            }
            self.bytes[start..stop].copy_from_slice(&buf[..stop - start]);
        }
        self.position = end;
        self.length = self.length.max(end);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for LimitedExportWriter {
    fn seek(&mut self, target: SeekFrom) -> std::io::Result<u64> {
        let next = match target {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => self.length.checked_add_signed(delta),
        };
        self.position = next.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid seek to a negative or overflowing position",
            )
        })?;
        Ok(self.position)
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
    use std::io::{Cursor, Read};
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
    fn sanitize_download_filename_keeps_non_latin_scripts() {
        // #647: every non-ASCII character used to become `-`.
        assert_eq!(
            sanitize_download_filename("Ελληνική σημείωση.md"),
            "Ελληνική σημείωση.md"
        );
        assert_eq!(sanitize_download_filename("中文笔记"), "中文笔记");
        assert_eq!(
            sanitize_download_filename("Straße Grüße ẞ"),
            "Straße Grüße ẞ"
        );
        assert_eq!(sanitize_download_filename("notes 🦀"), "notes 🦀");
    }

    #[test]
    fn sanitize_download_filename_drops_controls_quotes_and_direction_overrides() {
        assert_eq!(
            sanitize_download_filename("a\r\nb\"c\u{202E}gpj.md"),
            "ab-cgpj.md"
        );
    }

    #[test]
    fn content_disposition_carries_the_unicode_name_with_an_ascii_fallback() {
        let value = build_download_content_disposition("Ελληνική σημείωση.md");
        let (fallback, encoded) = value
            .strip_prefix("attachment; filename=\"")
            .and_then(|rest| rest.split_once("\"; filename*=UTF-8''"))
            .expect("both parameters");
        assert!(fallback.is_ascii());
        assert!(fallback.ends_with(".md"));
        let decoded = percent_decode(encoded);
        assert_eq!(decoded, "Ελληνική σημείωση.md");
    }

    #[test]
    fn content_disposition_fallback_never_carries_a_quote_or_line_break() {
        let value = build_download_content_disposition("a\"b\r\nc\\d.md");
        assert_eq!(
            value,
            "attachment; filename=\"a-b--c-d.md\"; filename*=UTF-8''a%22b%0D%0Ac%5Cd.md"
        );
        assert!(HeaderValue::from_str(&value).is_ok());
    }

    fn percent_decode(input: &str) -> String {
        let bytes = input.as_bytes();
        let mut out = Vec::new();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%' {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).expect("hex");
                out.push(u8::from_str_radix(hex, 16).expect("byte"));
                index += 3;
            } else {
                out.push(bytes[index]);
                index += 1;
            }
        }
        String::from_utf8(out).expect("utf-8 name")
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

        let export = build_note_export(vault, &note, |_| true).expect("export");

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
    fn build_note_export_leaves_out_a_refused_asset_like_a_missing_one() {
        // #342: the caller's surface check sees each asset by its contained
        // Vault-relative path, and a refusal changes nothing else.
        let dir = tempdir().expect("temp dir");
        let vault = dir.path();
        std::fs::create_dir_all(vault.join("Notes/private")).expect("notes dir");
        std::fs::write(vault.join("Notes/private/hidden.png"), b"hidden").expect("asset");
        let content = "# Home\n\n![[private/hidden.png]]\n![](../Notes/private/hidden.png)";
        let note = Note {
            title: "Home".to_string(),
            slug: "home".to_string(),
            relative_path: "Notes/Home".to_string(),
            content: content.to_string(),
            content_hash: "fnv1a64:0000000000000000".to_string(),
            layer: None,
            metadata: Default::default(),
        };

        let asked = std::cell::RefCell::new(Vec::new());
        let export = build_note_export(vault, &note, |path| {
            asked.borrow_mut().push(path.to_string());
            false
        })
        .expect("export");

        assert_eq!(asked.into_inner(), ["Notes/private/hidden.png"; 2]);
        assert_eq!(export.filename, "Home.md");
        assert_eq!(export.content_type, "text/markdown; charset=utf-8");
        assert_eq!(export.bytes, content.as_bytes());
    }

    #[test]
    fn build_note_export_bundles_an_asset_whose_size_suffix_pipe_is_escaped() {
        // #252: inside a table cell the pipe has to be written `\\|`, and the
        // export read the target as `diagram.png\\`, so the file was neither
        // bundled nor relinked and the note shipped with a dead image.
        let dir = tempdir().expect("temp dir");
        let vault = dir.path();
        let notes = vault.join("Notes");
        std::fs::create_dir_all(&notes).expect("notes dir");
        std::fs::write(notes.join("diagram.png"), b"png-bytes").expect("asset");
        let note = Note {
            title: "Home".to_string(),
            slug: "home".to_string(),
            relative_path: "Notes/Home".to_string(),
            content:
                "| pic |\n| --- |\n| ![[diagram.png\\|200]] |\n\n| link |\n| [[Plan\\|the plan]] |"
                    .to_string(),
            content_hash: "fnv1a64:0000000000000000".to_string(),
            layer: None,
            metadata: Default::default(),
        };

        let export = build_note_export(vault, &note, |_| true).expect("export");

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
            "| pic |\n| --- |\n| ![200](Home-assets/diagram.png) |\n\n| link |\n| the plan |"
        );

        let mut asset = Vec::new();
        archive
            .by_name("Home-assets/diagram.png")
            .expect("asset file")
            .read_to_end(&mut asset)
            .expect("asset bytes");
        assert_eq!(asset, b"png-bytes");
    }

    fn note_at(relative_path: &str, content: &str) -> Note {
        Note {
            title: relative_path.to_string(),
            slug: "note".to_string(),
            relative_path: relative_path.to_string(),
            content: content.to_string(),
            content_hash: "fnv1a64:0000000000000000".to_string(),
            layer: None,
            metadata: Default::default(),
        }
    }

    #[test]
    fn build_note_export_names_zip_entries_in_the_note_own_script() {
        let dir = tempdir().expect("temp dir");
        let vault = dir.path();
        std::fs::write(vault.join("σχέδιο.png"), b"png-bytes").expect("asset");
        let note = note_at("Ελληνική σημείωση.md", "![[σχέδιο.png]]");

        let export = build_note_export(vault, &note, |_| true).expect("export");

        assert_eq!(export.filename, "Ελληνική σημείωση.zip");
        let mut archive = zip::ZipArchive::new(Cursor::new(export.bytes)).expect("zip archive");
        let mut names = archive
            .file_names()
            .map(|name| name.expect("entry name").into_owned())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            [
                "Ελληνική σημείωση-assets/σχέδιο.png",
                "Ελληνική σημείωση.md"
            ]
        );
        assert!(
            archive
                .by_name("Ελληνική σημείωση-assets/σχέδιο.png")
                .is_ok()
        );
    }

    /// Bytes no compressor can shrink, from a fixed xorshift sequence.
    fn incompressible(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed | 1;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }

    fn export_two_assets(
        vault: &FsPath,
        one: &[u8],
        two: &[u8],
        maximum: usize,
    ) -> Result<Vec<u8>, ExportError> {
        std::fs::write(vault.join("one.bmp"), one).expect("asset");
        std::fs::write(vault.join("two.bmp"), two).expect("asset");
        let markdown = "![](one.bmp) ![](two.bmp)";
        let note = note_at("big.md", markdown);
        let assets = export_assets(vault, &note, markdown, "big.md", &|_: &str| true);
        assert_eq!(assets.len(), 2);
        build_zip_export("big.md", markdown, &assets, maximum)
    }

    #[test]
    fn an_archive_over_the_limit_is_too_large_not_a_failure() {
        // #647: two assets each under the limit whose archive passes it
        // surfaced as `Failed`, a 500 with an ERROR log.
        let dir = tempdir().expect("temp dir");
        let result = export_two_assets(
            dir.path(),
            &incompressible(40_000, 1),
            &incompressible(40_000, 2),
            60_000,
        );
        assert!(matches!(result, Err(ExportError::TooLarge)));
    }

    #[test]
    fn the_limit_counts_the_compressed_archive_not_the_raw_files() {
        // Two assets whose raw bytes pass the limit but which compress well
        // still download, as they did before #647.
        let dir = tempdir().expect("temp dir");
        let flat = vec![7u8; 40_000];
        let bytes = export_two_assets(dir.path(), &flat, &flat, 60_000).expect("fits compressed");
        assert!(bytes.len() <= 60_000);
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("a whole archive");
        let mut asset = Vec::new();
        archive
            .by_name("big-assets/two.bmp")
            .expect("asset entry")
            .read_to_end(&mut asset)
            .expect("asset bytes");
        assert_eq!(asset, flat);
    }

    #[test]
    fn an_archive_that_fits_is_whole_and_readable() {
        let dir = tempdir().expect("temp dir");
        let one = incompressible(20_000, 3);
        let bytes =
            export_two_assets(dir.path(), &one, &incompressible(20_000, 4), 60_000).expect("fits");
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("a whole archive");
        let mut asset = Vec::new();
        archive
            .by_name("big-assets/one.bmp")
            .expect("asset entry")
            .read_to_end(&mut asset)
            .expect("asset bytes");
        assert_eq!(asset, one);
    }

    #[test]
    fn limited_export_writer_keeps_only_its_bound_and_reports_the_overflow() {
        let mut writer = LimitedExportWriter::new(3);
        writer.write_all(b"abc").expect("fill limit");
        assert!(!writer.overflowed());
        writer
            .write_all(b"de")
            .expect("past the bound still answers");
        assert!(writer.overflowed());
        assert_eq!(writer.seek(SeekFrom::End(-1)).expect("seek"), 4);
        assert_eq!(writer.bytes, b"abc");
        assert!(matches!(writer.into_inner(), Err(ExportError::TooLarge)));
    }

    #[test]
    fn limited_export_writer_rewrites_in_place_after_a_seek() {
        let mut writer = LimitedExportWriter::new(8);
        writer.write_all(b"abcdef").expect("write");
        writer.seek(SeekFrom::Start(1)).expect("seek");
        writer.write_all(b"XY").expect("rewrite");
        writer.seek(SeekFrom::End(0)).expect("seek end");
        writer.write_all(b"g").expect("append");
        assert_eq!(writer.into_inner().ok().as_deref(), Some(&b"aXYdefg"[..]));
    }

    #[test]
    fn sanitize_download_filename_stays_within_a_filesystem_name() {
        let name = sanitize_download_filename(&"中".repeat(120));
        assert!(name.len() <= MAX_DOWNLOAD_NAME_BYTES);
        assert!(name.chars().all(|ch| ch == '中'));
        assert_eq!(sanitize_download_filename("a\u{200B}b\u{FEFF}c"), "abc");
    }

    #[test]
    fn sanitize_download_filename_keeps_link_safe_ascii() {
        // `#` or `%` in a zip entry name would break the archive's own links.
        assert_eq!(
            sanitize_download_filename("fig#1 100%.png"),
            "fig-1 100-.png"
        );
    }
}
