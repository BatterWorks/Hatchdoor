use std::collections::{HashMap, HashSet};
use std::fs;

use crate::cache::parse::for_non_code_line;

use super::markdown_links::{note_dir, note_link_destinations};
use super::paths::{normalize_link_target, normalize_title, slugify, split_wikilink_note_body};
use super::types::{NoteEntry, VaultIndex};

pub fn build_link_graph(
    index: &VaultIndex,
) -> (HashMap<String, Vec<String>>, HashMap<String, Vec<String>>) {
    let mut outgoing_by_slug: HashMap<String, Vec<String>> = HashMap::new();
    let mut backlinks_by_slug: HashMap<String, Vec<String>> = HashMap::new();
    let by_slug = &index.by_slug;

    for slug in &index.ordered_slugs {
        outgoing_by_slug.insert(slug.clone(), Vec::new());
        backlinks_by_slug.insert(slug.clone(), Vec::new());
    }

    for slug in &index.ordered_slugs {
        let Some(note) = by_slug.get(slug) else {
            continue;
        };

        let Ok(content) = fs::read_to_string(&note.path) else {
            continue;
        };

        let mut seen = HashSet::new();
        let mut outgoing = Vec::new();

        // Both forms of note link count alike (ADR-28), each through its own
        // resolver: a wikilink names a title, a Markdown link a path relative
        // to the note it was written in.
        let wikilinks = extract_wikilink_targets(&content)
            .into_iter()
            .filter_map(|target| {
                resolve_target_slug(&target, by_slug, &index.by_title, &index.by_path_title)
            });
        let folder = note_dir(&note.relative_path);
        let markdown_links = note_link_destinations(&content)
            .into_iter()
            .filter_map(|destination| index.resolve_note_link(destination, folder))
            .map(|entry| entry.slug.clone());

        for resolved_slug in wikilinks.chain(markdown_links) {
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
    for_non_code_line(content, |line| {
        extract_line_wikilink_targets(line, &mut targets);
    });
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
            let (target, _) = split_wikilink_note_body(body);
            if !target.is_empty() {
                targets.push(target.to_string());
            }
        }

        idx = end + 2;
    }
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
