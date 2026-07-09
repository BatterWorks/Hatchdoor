use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::vault::slugify;

#[derive(Debug)]
pub struct HeadingRow {
    pub level: usize,
    pub text: String,
    pub anchor: String,
    pub position: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSnapshot {
    pub mtime_ns: i64,
    pub size_bytes: i64,
}

pub fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub fn file_snapshot(path: &Path) -> Result<FileSnapshot, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("failed reading metadata for '{}': {error}", path.display()))?;
    let modified = metadata.modified().map_err(|error| {
        format!(
            "failed reading modified time for '{}': {error}",
            path.display()
        )
    })?;
    let mtime_ns = modified
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or_default();
    let size_bytes = metadata.len().min(i64::MAX as u64) as i64;

    Ok(FileSnapshot {
        mtime_ns,
        size_bytes,
    })
}

pub fn content_hash(content: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("fnv1a64:{hash:016x}")
}

pub fn extract_headings(content: &str) -> Vec<HeadingRow> {
    content
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let trimmed = line.trim_start();
            let level = trimmed.chars().take_while(|ch| *ch == '#').count();
            if level == 0
                || level > 6
                || !trimmed.chars().nth(level).is_some_and(char::is_whitespace)
            {
                return None;
            }
            let text = trimmed[level..].trim().to_string();
            if text.is_empty() {
                return None;
            }
            Some(HeadingRow {
                level,
                anchor: slugify(&text),
                text,
                position: idx,
            })
        })
        .collect()
}

pub fn extract_tags(content: &str) -> HashSet<String> {
    let mut tags = HashSet::new();
    let (frontmatter, body) = split_frontmatter(content);
    extract_frontmatter_tags(frontmatter, &mut tags);
    extract_inline_tags(body, &mut tags);
    tags
}

fn split_frontmatter(content: &str) -> (&str, &str) {
    let lines: Vec<&str> = content.splitn(3, '\n').collect();
    if lines.len() < 2 || lines[0].trim() != "---" {
        return ("", content);
    }
    let rest = &content[lines[0].len() + 1..];
    if let Some(end) = rest.find("\n---") {
        let fm_end = lines[0].len() + 1 + end;
        let body_start = fm_end + 4; // skip "\n---"
        let body = if body_start < content.len() {
            &content[body_start..]
        } else {
            ""
        };
        (&content[lines[0].len() + 1..fm_end], body)
    } else {
        ("", content)
    }
}

fn extract_frontmatter_tags(frontmatter: &str, tags: &mut HashSet<String>) {
    // Find a line starting with "tags:"
    let mut in_tags = false;
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("tags:") {
            in_tags = true;
            // Inline array form: tags: [a, b, c]
            let rest = rest.trim();
            if rest.starts_with('[') {
                let inner = rest.trim_matches(|c| c == '[' || c == ']');
                for item in inner.split(',') {
                    push_tag(item.trim().trim_matches('"').trim_matches('\''), tags);
                }
                in_tags = false; // inline array is self-contained
            }
            // else: block sequence follows on subsequent lines
        } else if in_tags {
            // Block sequence item: "  - tagname"
            if let Some(item) = trimmed.strip_prefix("- ") {
                push_tag(item.trim().trim_matches('"').trim_matches('\''), tags);
            } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
                // Hit a non-list line — tags block is over
                in_tags = false;
            }
        }
    }
}

fn push_tag(raw: &str, tags: &mut HashSet<String>) {
    let cleaned: String = raw
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '/'))
        .collect();
    if !cleaned.is_empty() {
        tags.insert(cleaned.to_lowercase());
    }
}

fn extract_inline_tags(body: &str, tags: &mut HashSet<String>) {
    for token in body.split_whitespace() {
        let token = token.trim_matches(|ch: char| {
            matches!(
                ch,
                ',' | '.' | ';' | ':' | '!' | '?' | ')' | '(' | '[' | ']' | '{' | '}'
            )
        });
        let Some(tag) = token.strip_prefix('#') else {
            continue;
        };
        if tag.is_empty() || tag.starts_with('#') || tag.chars().all(|ch| ch == '-') {
            continue;
        }
        // Inline tags must be namespaced (e.g. #area/health), not free-form words or bare numbers.
        let slash = tag.find('/');
        if !slash.is_some_and(|pos| pos > 0 && pos < tag.len() - 1) {
            continue;
        }
        push_tag(tag, tags);
    }
}

pub fn build_fts_query(input: &str) -> Option<String> {
    let tokens = fts_query_terms(input)
        .into_iter()
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" OR "))
    }
}

pub fn fts_query_terms(input: &str) -> Vec<String> {
    input
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-'))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_headings_tags_and_fts_query_tokens() {
        let headings = extract_headings("# One\ntext\n### Three");
        assert_eq!(headings.len(), 2);
        assert_eq!(headings[0].anchor, "one");
        let tags = extract_tags("hello #topic/network #dns/network, ## no");
        assert!(tags.contains("topic/network"));
        assert!(tags.contains("dns/network"));
        assert_eq!(
            build_fts_query("réseau dns"),
            Some("\"réseau\" OR \"dns\"".to_string())
        );
    }

    #[test]
    fn extracts_frontmatter_tags_inline_array() {
        let content = "---\ntags: [type/reference, topic/api, topic/foo-bar]\ncreated: 2026-01-01\n---\n\nBody text.";
        let tags = extract_tags(content);
        assert!(tags.contains("type/reference"), "missing type/reference");
        assert!(tags.contains("topic/api"), "missing topic/api");
        assert!(tags.contains("topic/foo-bar"), "missing topic/foo-bar");
    }

    #[test]
    fn extracts_frontmatter_tags_block_sequence() {
        let content =
            "---\ntags:\n  - programming\n  - philosophy\ncreated: 2026-01-01\n---\n\nBody text.";
        let tags = extract_tags(content);
        assert!(tags.contains("programming"), "missing programming");
        assert!(tags.contains("philosophy"), "missing philosophy");
    }

    #[test]
    fn inline_hashtags_require_namespace() {
        let content = "---\ntags: [existing]\n---\n\nSome text with #area/health here but not #freeform or #1 or #0599.";
        let tags = extract_tags(content);
        assert!(tags.contains("existing"), "frontmatter tag preserved");
        assert!(
            tags.contains("area/health"),
            "namespaced inline tag accepted"
        );
        assert!(!tags.contains("freeform"), "free-form inline tag rejected");
        assert!(!tags.contains("1"), "numeric inline tag rejected");
        assert!(!tags.contains("0599"), "numeric inline tag rejected");
    }

    #[test]
    fn content_hash_is_stable_and_namespaced() {
        assert_eq!(content_hash("abc"), "fnv1a64:e71fa2190541574b");
        assert_ne!(content_hash("abc"), content_hash("abd"));
    }
}
