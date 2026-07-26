#[allow(dead_code)]
pub struct FrontmatterMetadata {
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
}

#[allow(dead_code)]
pub fn strip_frontmatter(content: &str) -> &str {
    if !content.starts_with("---") {
        return content;
    }
    let after_open = match content.strip_prefix("---") {
        Some(rest) => rest,
        None => return content,
    };
    let after_open = after_open.trim_start_matches(['\r']);
    let after_open = match after_open.strip_prefix('\n') {
        Some(rest) => rest,
        None => return content,
    };
    let mut search_from = 0;
    while let Some(idx) = after_open[search_from..].find("\n---") {
        let abs = search_from + idx + 1;
        let end_marker = &after_open[abs..];
        let after_marker = end_marker.strip_prefix("---").unwrap_or(end_marker);
        let after_marker = after_marker.trim_start_matches(['\r']);
        if after_marker.is_empty() || after_marker.starts_with('\n') {
            return after_marker.trim_start_matches(['\r', '\n']);
        }
        search_from = abs + 3;
    }
    content
}

/// Result of [`strip_code_fences`]: the normalized body with fence marker lines
/// removed but fence *contents* preserved, plus the byte ranges within `text`
/// that fall inside fenced code blocks.
#[allow(dead_code)]
pub struct Normalized {
    pub text: String,
    /// Byte ranges in `text` that lie inside fenced code blocks (marker lines
    /// excluded). Sorted ascending and non-overlapping. Heading derivation must
    /// skip lines whose start falls inside one of these, otherwise a
    /// `#!/usr/bin/env bash` shebang or `#` comment inside a fence is mistaken
    /// for an ATX heading.
    pub fenced: Vec<std::ops::Range<usize>>,
}

/// Returns true if `pos` (a byte offset into `Normalized::text`) lies inside a
/// fenced code block.
#[allow(dead_code)]
pub fn in_fenced(fenced: &[std::ops::Range<usize>], pos: usize) -> bool {
    fenced.iter().any(|r| r.contains(&pos))
}

#[allow(dead_code)]
pub fn strip_code_fences(content: &str) -> Normalized {
    let mut out = String::with_capacity(content.len());
    let mut fenced = Vec::new();
    // `Some(start)` while inside a fence; `start` is the offset in `out` of the
    // first content byte after the opening marker.
    let mut fence_start: Option<usize> = None;
    for line in content.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            match fence_start.take() {
                Some(start) => fenced.push(start..out.len()),
                None => fence_start = Some(out.len()),
            }
            continue;
        }
        out.push_str(line);
    }
    if let Some(start) = fence_start {
        // Unterminated fence: treat the remainder as fenced.
        fenced.push(start..out.len());
    }
    Normalized { text: out, fenced }
}

#[allow(dead_code)]
pub fn extract_frontmatter_metadata(content: &str) -> FrontmatterMetadata {
    let mut tags = Vec::new();
    let mut aliases = Vec::new();
    if !content.starts_with("---") {
        return FrontmatterMetadata { tags, aliases };
    }
    let after_open = content
        .strip_prefix("---")
        .unwrap_or("")
        .trim_start_matches(['\r', '\n']);
    let end = match after_open.find("\n---") {
        Some(idx) => idx,
        None => return FrontmatterMetadata { tags, aliases },
    };
    let block = &after_open[..end];
    parse_simple_yaml_list(block, "tags", &mut tags);
    parse_simple_yaml_list(block, "aliases", &mut aliases);
    FrontmatterMetadata { tags, aliases }
}

#[allow(dead_code)]
fn parse_simple_yaml_list(block: &str, key: &str, out: &mut Vec<String>) {
    let mut lines = block.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();
        let stripped = match trimmed.strip_prefix(key) {
            Some(rest) => rest,
            None => continue,
        };
        let rest = stripped.trim_start();
        if !rest.starts_with(':') {
            continue;
        }
        let value = rest[1..].trim();
        if value.starts_with('[') && value.ends_with(']') {
            let inner = &value[1..value.len() - 1];
            for item in inner.split(',') {
                let item = item.trim().trim_matches(|c| c == '"' || c == '\'');
                if !item.is_empty() {
                    out.push(item.to_string());
                }
            }
            return;
        }
        if value.is_empty() {
            while let Some(next) = lines.peek() {
                let next_trim = next.trim_start();
                if let Some(item) = next_trim.strip_prefix("- ") {
                    out.push(
                        item.trim()
                            .trim_matches(|c| c == '"' || c == '\'')
                            .to_string(),
                    );
                    lines.next();
                } else {
                    break;
                }
            }
            return;
        }
        out.push(value.trim_matches(|c| c == '"' || c == '\'').to_string());
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_frontmatter_removes_yaml_block_at_start() {
        let input = "---\ntitle: Foo\ntags: [a, b]\n---\n\n# Heading\n\nBody.";
        assert_eq!(strip_frontmatter(input), "# Heading\n\nBody.");
    }

    #[test]
    fn strip_frontmatter_leaves_content_without_frontmatter_untouched() {
        let input = "# Heading\n\nBody.";
        assert_eq!(strip_frontmatter(input), input);
    }

    #[test]
    fn strip_frontmatter_ignores_yaml_block_not_at_start() {
        let input = "# Heading\n\n---\nnot frontmatter\n---";
        assert_eq!(strip_frontmatter(input), input);
    }

    #[test]
    fn strip_code_fences_removes_fence_lines_keeps_contents() {
        let input = "before\n```rust\nfn foo() {}\n```\nafter";
        let result = strip_code_fences(input);
        assert_eq!(result.text, "before\nfn foo() {}\nafter");
    }

    #[test]
    fn strip_code_fences_reports_fenced_content_ranges() {
        let input = "before\n```rust\nfn foo() {}\n```\nafter";
        let result = strip_code_fences(input);
        // The fenced range covers exactly "fn foo() {}\n" within the output.
        assert_eq!(result.fenced.len(), 1);
        let r = result.fenced[0].clone();
        assert_eq!(&result.text[r], "fn foo() {}\n");
    }

    #[test]
    fn strip_code_fences_handles_unterminated_fence() {
        let input = "before\n```rust\nfn foo() {}\n";
        let result = strip_code_fences(input);
        assert_eq!(result.text, "before\nfn foo() {}\n");
        assert_eq!(result.fenced.len(), 1);
        assert!(in_fenced(
            &result.fenced,
            result.text.find("fn foo").unwrap()
        ));
    }

    #[test]
    fn extract_tags_and_aliases_pulls_from_yaml_frontmatter() {
        let input = "---\ntags: [project, hatchdoor]\naliases:\n  - hd\n  - door\n---\nbody";
        let meta = extract_frontmatter_metadata(input);
        assert_eq!(meta.tags, vec!["project", "hatchdoor"]);
        assert_eq!(meta.aliases, vec!["hd", "door"]);
    }

    #[test]
    fn extract_tags_and_aliases_returns_empty_for_no_frontmatter() {
        let meta = extract_frontmatter_metadata("just body");
        assert!(meta.tags.is_empty());
        assert!(meta.aliases.is_empty());
    }
}
