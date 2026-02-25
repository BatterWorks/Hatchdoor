use std::collections::{HashMap, HashSet};
use std::fs;

use super::paths::{normalize_link_target, normalize_title, slugify};
use super::types::NoteEntry;

pub(crate) fn build_link_graph(
    by_slug: &HashMap<String, NoteEntry>,
    by_title: &HashMap<String, String>,
    by_path_title: &HashMap<String, String>,
    ordered_slugs: &[String],
) -> (HashMap<String, Vec<String>>, HashMap<String, Vec<String>>) {
    let mut outgoing_by_slug: HashMap<String, Vec<String>> = HashMap::new();
    let mut backlinks_by_slug: HashMap<String, Vec<String>> = HashMap::new();

    for slug in ordered_slugs {
        outgoing_by_slug.insert(slug.clone(), Vec::new());
        backlinks_by_slug.insert(slug.clone(), Vec::new());
    }

    for slug in ordered_slugs {
        let Some(note) = by_slug.get(slug) else {
            continue;
        };

        let Ok(content) = fs::read_to_string(&note.path) else {
            continue;
        };

        let mut seen = HashSet::new();
        let mut outgoing = Vec::new();

        for target in extract_wikilink_targets(&content) {
            let Some(resolved_slug) =
                resolve_target_slug(&target, by_slug, by_title, by_path_title)
            else {
                continue;
            };

            if resolved_slug == note.slug || !seen.insert(resolved_slug.clone()) {
                continue;
            }

            outgoing.push(resolved_slug.clone());
            backlinks_by_slug
                .entry(resolved_slug)
                .or_default()
                .push(note.slug.clone());
        }

        outgoing_by_slug.insert(note.slug.clone(), outgoing);
    }

    for links in outgoing_by_slug.values_mut() {
        sort_slug_links(links, by_slug);
    }
    for links in backlinks_by_slug.values_mut() {
        links.sort();
        links.dedup();
        sort_slug_links(links, by_slug);
    }

    (outgoing_by_slug, backlinks_by_slug)
}

fn sort_slug_links(links: &mut [String], by_slug: &HashMap<String, NoteEntry>) {
    links.sort_by(|left, right| {
        let left_path = by_slug
            .get(left)
            .map(|entry| entry.relative_path.as_str())
            .unwrap_or("");
        let right_path = by_slug
            .get(right)
            .map(|entry| entry.relative_path.as_str())
            .unwrap_or("");
        left_path.cmp(right_path)
    });
}

fn extract_wikilink_targets(content: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut fenced_marker: Option<(u8, usize)> = None;

    for line in content.lines() {
        let trimmed = line.trim_start();

        if let Some((marker, min_len)) = fenced_marker {
            if let Some((close_marker, close_len)) = parse_fence_marker(trimmed)
                && close_marker == marker
                && close_len >= min_len
            {
                fenced_marker = None;
            }
            continue;
        }

        if let Some(marker) = parse_fence_marker(trimmed) {
            fenced_marker = Some(marker);
            continue;
        }

        let no_inline_code = strip_inline_code_segments(line);
        extract_line_wikilink_targets(&no_inline_code, &mut targets);
    }

    targets
}

fn extract_line_wikilink_targets(line: &str, targets: &mut Vec<String>) {
    let bytes = line.as_bytes();
    let mut idx = 0usize;

    while idx + 1 < bytes.len() {
        if bytes[idx] != b'[' || bytes[idx + 1] != b'[' {
            idx += 1;
            continue;
        }

        let is_embed = idx > 0 && bytes[idx - 1] == b'!';
        let mut end = idx + 2;

        while end + 1 < bytes.len() {
            if bytes[end] == b']' && bytes[end + 1] == b']' {
                break;
            }
            end += 1;
        }

        if end + 1 >= bytes.len() {
            break;
        }

        if !is_embed {
            let body = &line[idx + 2..end];
            let target = parse_wikilink_target(body);
            if !target.is_empty() {
                targets.push(target);
            }
        }

        idx = end + 2;
    }
}

fn parse_fence_marker(trimmed_line: &str) -> Option<(u8, usize)> {
    let bytes = trimmed_line.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let marker = bytes[0];
    if marker != b'`' && marker != b'~' {
        return None;
    }

    let mut len = 1usize;
    while len < bytes.len() && bytes[len] == marker {
        len += 1;
    }

    if len >= 3 { Some((marker, len)) } else { None }
}

fn strip_inline_code_segments(line: &str) -> String {
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

            if inline_marker_len == 0 {
                inline_marker_len = marker_len;
            } else if marker_len == inline_marker_len {
                inline_marker_len = 0;
            }

            idx += marker_len;
            continue;
        }

        if inline_marker_len == 0 {
            out.push(chars[idx]);
        }
        idx += 1;
    }

    out
}

fn parse_wikilink_target(body: &str) -> String {
    let before_alias = body.split('|').next().unwrap_or(body).trim();
    let before_heading = before_alias
        .split('#')
        .next()
        .unwrap_or(before_alias)
        .trim();
    before_heading
        .split('^')
        .next()
        .unwrap_or(before_heading)
        .trim()
        .to_string()
}

fn resolve_target_slug(
    raw_target: &str,
    by_slug: &HashMap<String, NoteEntry>,
    by_title: &HashMap<String, String>,
    by_path_title: &HashMap<String, String>,
) -> Option<String> {
    let normalized_target = normalize_link_target(raw_target);

    if let Some(slug) = by_path_title.get(&normalize_title(&normalized_target)) {
        return Some(slug.clone());
    }

    let base = normalized_target
        .rsplit('/')
        .next()
        .unwrap_or(&normalized_target);

    if let Some(slug) = by_title.get(&normalize_title(base)) {
        return Some(slug.clone());
    }

    let slug = slugify(base);
    if by_slug.contains_key(&slug) {
        return Some(slug);
    }

    None
}
