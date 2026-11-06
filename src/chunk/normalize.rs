use crate::cache::parse::parse_fence_marker;

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
    /// excluded). Sorted ascending and non-overlapping.
    pub fenced: Vec<std::ops::Range<usize>>,
    /// The opening marker line of each range in `fenced`, in the same order,
    /// without its indentation or line ending (for example "````mermaid"). An
    /// empty block still has an entry here, so the two stay in step.
    pub openers: Vec<String>,
    /// Byte ranges of the input that were removed: each fence marker line,
    /// line ending included. Sorted ascending.
    removed: Vec<std::ops::Range<usize>>,
}

impl Normalized {
    /// Where byte `offset` of the input to [`strip_code_fences`] lands in
    /// `text`. An offset inside a removed marker line maps to where that line
    /// was.
    pub fn text_offset(&self, offset: usize) -> usize {
        let removed: usize = self
            .removed
            .iter()
            .take_while(|range| range.start < offset)
            .map(|range| range.end.min(offset) - range.start)
            .sum();
        offset - removed
    }
}

/// The fence marker `line` holds, as its character and run length, or `None`
/// when it holds none. A marker is a run of three or more backticks or tildes
/// after at most three spaces of indentation, never a tab: the rule the
/// note's Sections use (`vault::sections`), so a block that ends there ends
/// here too. A block opened by one marker is closed only by a run of the same
/// character at least as long. Shared by chunking and by a reader of
/// `Chunk::stored_content`, so they cannot drift apart.
pub fn fence_marker(line: &str) -> Option<(u8, usize)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() >= 4 || trimmed.starts_with('\t') {
        return None;
    }
    parse_fence_marker(trimmed)
}

/// Whether `line` closes a block opened by `opener` (a [`fence_marker`]
/// result).
pub fn closes_fence(opener: (u8, usize), line: &str) -> bool {
    fence_marker(line).is_some_and(|(marker, len)| marker == opener.0 && len >= opener.1)
}

/// `text[start..end]` with a fence marker line put back around every part of
/// it that lies inside a fenced block. A block cut by either end of the slice
/// is closed and reopened, so the result always holds whole blocks. The
/// opening marker is the block's own opening line, info string included, and
/// the closing one a run of the same character and length, so a block that
/// shows a shorter fence inside it stays one block.
pub fn restore_fences(normalized: &Normalized, start: usize, end: usize) -> String {
    let text = &normalized.text;
    let mut out = String::with_capacity(end - start);
    let mut cursor = start;
    for (range, opener) in normalized.fenced.iter().zip(&normalized.openers) {
        let (from, to) = (range.start.max(start), range.end.min(end));
        if from >= to || text[from..to].trim().is_empty() {
            continue;
        }
        out.push_str(&text[cursor..from]);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(opener);
        out.push('\n');
        out.push_str(&text[from..to]);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        let run = opener.len() - opener.trim_start_matches(['`', '~']).len();
        out.push_str(&opener[..run]);
        if to < end {
            out.push('\n');
        }
        cursor = to;
    }
    out.push_str(&text[cursor..end]);
    out
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
    let mut openers = Vec::new();
    let mut removed = Vec::new();
    // While inside a fence: the opening marker, and the offset in `out` of
    // the first content byte after it.
    let mut open: Option<((u8, usize), usize)> = None;
    let mut offset = 0usize;
    for line in content.split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        let marker_line = match open {
            Some((opener, start)) if closes_fence(opener, line) => {
                fenced.push(start..out.len());
                open = None;
                true
            }
            Some(_) => false,
            None => match fence_marker(line) {
                Some(opener) => {
                    open = Some((opener, out.len()));
                    openers.push(line.trim().to_string());
                    true
                }
                None => false,
            },
        };
        if marker_line {
            removed.push(line_start..offset);
        } else {
            out.push_str(line);
        }
    }
    if let Some((_, start)) = open {
        // Unterminated fence: treat the remainder as fenced.
        fenced.push(start..out.len());
    }
    Normalized {
        text: out,
        fenced,
        openers,
        removed,
    }
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
    fn a_fence_closes_only_at_a_run_of_its_own_character_at_least_as_long() {
        let input = "a\n````markdown\n```js\nx\n```\n````\nb\n~~~bash\n# c\n```\n~~~~\nd\n";
        let result = strip_code_fences(input);
        assert_eq!(result.text, "a\n```js\nx\n```\nb\n# c\n```\nd\n");
        let fenced: Vec<&str> = result
            .fenced
            .iter()
            .map(|r| &result.text[r.clone()])
            .collect();
        assert_eq!(fenced, vec!["```js\nx\n```\n", "# c\n```\n"]);
        assert_eq!(result.openers, vec!["````markdown", "~~~bash"]);
        assert_eq!(
            restore_fences(&result, 0, result.text.len()),
            "a\n````markdown\n```js\nx\n```\n````\nb\n~~~bash\n# c\n```\n~~~\nd\n"
        );
    }

    #[test]
    fn an_indented_or_tab_led_marker_is_no_fence() {
        let input = "a\n    ```\nb\n\t```\nc\n   ```\nd\n";
        let result = strip_code_fences(input);
        assert_eq!(result.text, "a\n    ```\nb\n\t```\nc\nd\n");
        assert_eq!(result.fenced.len(), 1);
    }

    #[test]
    fn text_offset_maps_an_input_offset_past_removed_marker_lines() {
        let input = "a\n```\nx\n```\nb\n";
        let result = strip_code_fences(input);
        let b = input.find('b').unwrap();
        assert_eq!(&result.text[result.text_offset(b)..], "b\n");
        assert_eq!(result.text_offset(0), 0);
    }

    #[test]
    fn restore_fences_puts_the_markers_back_with_the_info_string() {
        let input = "before\n```rust\nfn foo() {}\n```\nafter";
        let normalized = strip_code_fences(input);
        let restored = restore_fences(&normalized, 0, normalized.text.len());
        assert_eq!(restored, input);
    }

    #[test]
    fn restore_fences_closes_and_reopens_a_block_cut_by_the_slice() {
        let normalized = strip_code_fences("intro\n````mermaid\none\ntwo\n````\noutro");
        let cut = normalized.text.find("two").unwrap();
        assert_eq!(
            restore_fences(&normalized, 0, cut),
            "intro\n````mermaid\none\n````"
        );
        assert_eq!(
            restore_fences(&normalized, cut, normalized.text.len()),
            "````mermaid\ntwo\n````\noutro"
        );
    }

    #[test]
    fn restore_fences_closes_an_unterminated_block_and_skips_an_empty_one() {
        let normalized = strip_code_fences("a\n```\n```\nb\n```sh\nls");
        assert_eq!(
            restore_fences(&normalized, 0, normalized.text.len()),
            "a\nb\n```sh\nls\n```"
        );
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
