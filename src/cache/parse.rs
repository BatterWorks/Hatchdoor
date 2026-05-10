use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::vault::slugify;

#[derive(Debug)]
pub(crate) struct HeadingRow {
    pub(crate) level: usize,
    pub(crate) text: String,
    pub(crate) anchor: String,
    pub(crate) position: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileSnapshot {
    pub(crate) mtime_ns: i64,
    pub(crate) size_bytes: i64,
}

pub(crate) fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub(crate) fn file_snapshot(path: &Path) -> Result<FileSnapshot, String> {
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

pub(crate) fn content_hash(content: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("fnv1a64:{hash:016x}")
}

pub(crate) fn extract_headings(content: &str) -> Vec<HeadingRow> {
    content
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let trimmed = line.trim_start();
            let level = trimmed.chars().take_while(|ch| *ch == '#').count();
            if level == 0 || level > 6 || !trimmed.chars().nth(level).is_some_and(char::is_whitespace)
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

pub(crate) fn extract_tags(content: &str) -> HashSet<String> {
    content
        .split_whitespace()
        .filter_map(|token| {
            let token = token.trim_matches(|ch: char| {
                matches!(ch, ',' | '.' | ';' | ':' | '!' | '?' | ')' | '(' | '[' | ']' | '{' | '}')
            });
            let tag = token.strip_prefix('#')?;
            if tag.is_empty() || tag.starts_with('#') || tag.chars().all(|ch| ch == '-') {
                return None;
            }
            let cleaned = tag
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '/'))
                .collect::<String>();
            if cleaned.is_empty() {
                None
            } else {
                Some(cleaned.to_lowercase())
            }
        })
        .collect()
}

pub(crate) fn build_fts_query(input: &str) -> Option<String> {
    let tokens = input
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-'))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" OR "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_headings_tags_and_fts_query_tokens() {
        let headings = extract_headings("# One\ntext\n### Three");
        assert_eq!(headings.len(), 2);
        assert_eq!(headings[0].anchor, "one");
        let tags = extract_tags("hello #HomeLab #dns/network, ## no");
        assert!(tags.contains("homelab"));
        assert!(tags.contains("dns/network"));
        assert_eq!(
            build_fts_query("réseau dns"),
            Some("\"réseau\" OR \"dns\"".to_string())
        );
    }

    #[test]
    fn content_hash_is_stable_and_namespaced() {
        assert_eq!(content_hash("abc"), "fnv1a64:e71fa2190541574b");
        assert_ne!(content_hash("abc"), content_hash("abd"));
    }
}
