use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::vault::slugify;

#[derive(Debug)]
pub(crate) struct HeadingRow {
    pub(crate) level: usize,
    pub(crate) text: String,
    pub(crate) anchor: String,
    pub(crate) position: usize,
}

pub(crate) fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub(crate) fn content_hash(content: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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
        Some(tokens.join(" "))
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
            Some("\"réseau\" \"dns\"".to_string())
        );
    }
}
