use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tracing::warn;

use crate::api_types::ErrorResponse;
use crate::app_state::{AppState, sqlite_cache};
use crate::vault::Note;

pub(crate) async fn note_download_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };

    let note = match cache.read_note_by_slug(&slug) {
        Ok(Some(note)) => note,
        Ok(None) => {
            warn!(slug = %slug, "Note not found for download");
            return note_not_found_response(&slug);
        }
        Err(error) => {
            return internal_error_response(format!("Failed reading note {slug}: {error}"));
        }
    };

    let filename = download_filename_for_note(&note);
    let content_disposition = build_download_content_disposition(&filename);

    let mut response = Response::new(clean_markdown_export(&note.content).into());
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
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

fn note_not_found_response(slug: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: format!("Note not found: {slug}"),
        }),
    )
        .into_response()
}

fn internal_error_response(error: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error }),
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
}
