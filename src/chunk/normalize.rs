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

#[allow(dead_code)]
pub fn strip_code_fences(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    for line in content.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            continue;
        }
        out.push_str(line);
    }
    out
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
        assert_eq!(strip_code_fences(input), "before\nfn foo() {}\nafter");
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
