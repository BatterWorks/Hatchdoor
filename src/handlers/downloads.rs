use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Write};
use std::path::{Component, Path as FsPath, PathBuf};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tracing::warn;
use zip::write::SimpleFileOptions;

use crate::api_types::ErrorResponse;
use crate::app_state::{AppState, sqlite_cache, vault_unavailable};
use crate::vault::Note;

pub async fn note_download_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    // Reading the note and bundling its assets is filesystem + zip work; keep it
    // off the async runtime.
    let lookup_slug = slug.clone();
    let Some(vault_path) = state.vault_path().await else {
        return vault_unavailable().into_response();
    };
    let demo_mode = state.demo_mode;
    let export = match crate::app_state::run_blocking(move || {
        let Some(note) = cache.read_note_by_slug(&lookup_slug)? else {
            return Ok(None);
        };
        // In demo mode a demoted note is excluded outright — downloading it must
        // 404 like a missing note, not stream demoted content to the public.
        if demo_mode && note.layer.is_some() {
            return Ok(None);
        }
        build_note_export(&vault_path, &note).map(Some)
    })
    .await
    {
        Ok(Some(export)) => export,
        Ok(None) => {
            warn!(slug = %slug, "Note not found for download");
            return note_not_found_response(&slug);
        }
        Err(err) => return err.into_response(),
    };
    let filename = export.filename;
    let content_disposition = build_download_content_disposition(&filename);

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

struct NoteExport {
    filename: String,
    content_type: &'static str,
    bytes: Vec<u8>,
}

fn note_not_found_response(slug: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: format!("Note not found: {slug}"),
        }),
    )
        .into_response()
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

fn build_note_export(vault_root: &FsPath, note: &Note) -> Result<NoteExport, String> {
    let markdown_filename = download_filename_for_note(note);
    let markdown = clean_markdown_export(&note.content);
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

fn build_download_content_disposition(filename: &str) -> String {
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
) -> Result<Vec<u8>, String> {
    let cursor = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    writer
        .start_file(markdown_filename, options)
        .map_err(|error| format!("failed to add markdown to export zip: {error}"))?;
    writer
        .write_all(markdown.as_bytes())
        .map_err(|error| format!("failed to write markdown to export zip: {error}"))?;

    for asset in assets {
        let bytes = std::fs::read(&asset.source_path).map_err(|error| {
            format!(
                "failed reading asset '{}' for export: {error}",
                asset.source_path.display()
            )
        })?;
        writer
            .start_file(&asset.zip_path, options)
            .map_err(|error| format!("failed to add asset to export zip: {error}"))?;
        writer.write_all(&bytes).map_err(|error| {
            format!(
                "failed to write asset '{}' to export zip: {error}",
                asset.source_path.display()
            )
        })?;
    }

    writer
        .finish()
        .map(|cursor| cursor.into_inner())
        .map_err(|error| format!("failed to finish export zip: {error}"))
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

fn percent_encode_filename(input: &str) -> String {
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
}
