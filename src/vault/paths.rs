use std::collections::HashMap;
use std::path::Path;

use super::types::NoteEntry;

pub fn unique_slug(base: &str, by_slug: &HashMap<String, NoteEntry>) -> String {
    if !by_slug.contains_key(base) {
        return base.to_string();
    }

    let mut idx = 2usize;
    loop {
        let candidate = format!("{base}-{idx}");
        if !by_slug.contains_key(&candidate) {
            return candidate;
        }
        idx += 1;
    }
}

pub fn normalize_title(input: &str) -> String {
    input.trim().to_lowercase()
}

pub fn strip_md_extension(input: &str) -> &str {
    input.strip_suffix(".md").unwrap_or(input)
}

pub fn normalize_link_target(input: &str) -> String {
    strip_md_extension(input.trim()).replace('\\', "/")
}

pub fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;

    for c in input.trim().chars() {
        let mapped = if c.is_ascii_alphanumeric() {
            c.to_ascii_lowercase()
        } else if c == ' ' || c == '-' || c == '_' {
            '-'
        } else {
            continue;
        };

        if mapped == '-' {
            if !prev_dash && !out.is_empty() {
                out.push('-');
            }
            prev_dash = true;
        } else {
            out.push(mapped);
            prev_dash = false;
        }
    }

    while out.ends_with('-') {
        out.pop();
    }

    out
}

/// Extensions the Vault asset route will serve. The asset index and
/// `handlers::assets` share this list so wikilink resolution can never name a
/// path the route would then refuse: a resolved embed that 404s is worse than
/// an unresolved one, because it looks like a broken file rather than a
/// broken link.
pub fn servable_asset_extensions() -> &'static [&'static str] {
    &[
        "png", "jpg", "jpeg", "gif", "webp", "svg", "avif", "bmp", "pdf",
    ]
}

pub fn is_servable_asset(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            servable_asset_extensions().contains(&extension.to_ascii_lowercase().as_str())
        })
}

pub fn relative_note_path_without_ext(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let as_string = relative.to_str()?.replace('\\', "/");
    Some(strip_md_extension(&as_string).to_string())
}

pub fn content_snippet(content: &str, normalized_query: &str) -> Option<String> {
    content
        .lines()
        .find(|line| normalize_title(line).contains(normalized_query))
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.chars().count() > 180 {
                let shortened: String = trimmed.chars().take(177).collect();
                format!("{shortened}...")
            } else {
                trimmed.to_string()
            }
        })
}
