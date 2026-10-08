//! The one rule for what a Markdown section is. `replace_section` writes by it
//! and the read core's outline and section reads answer by it, so a section
//! read back is exactly the span a write would replace (#502).

use std::ops::Range;

use crate::cache::parse::parse_fence_marker;

/// One ATX heading outside any fenced code block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SectionHeading<'a> {
    /// Byte offset of the heading's line in the scanned content.
    pub(crate) offset: usize,
    /// 1 to 6.
    pub(crate) level: usize,
    /// The heading line with its `#` characters, trimmed at both ends. This
    /// is the spelling `replace_section` matches.
    pub(crate) line: &'a str,
}

impl<'a> SectionHeading<'a> {
    /// The heading without its `#` characters: one component of a heading
    /// path.
    pub(crate) fn text(&self) -> &'a str {
        self.line[self.level..].trim()
    }
}

/// A note's headings in document order, and the section each one opens.
#[derive(Debug)]
pub(crate) struct NoteSections<'a> {
    content_len: usize,
    headings: Vec<SectionHeading<'a>>,
}

impl<'a> NoteSections<'a> {
    /// Every heading in `content`.
    pub(crate) fn scan(content: &'a str) -> Self {
        Self::scan_from(content, 0)
    }

    /// Every heading at or after byte `start`, which must sit on a line
    /// boundary. A caller that knows where the frontmatter block ends passes
    /// that, so a `# comment` in the YAML is never read as a heading. Offsets
    /// stay relative to the whole of `content`.
    pub(crate) fn scan_from(content: &'a str, start: usize) -> Self {
        let mut headings = Vec::new();
        let mut fenced_marker: Option<(u8, usize)> = None;
        let mut offset = start;
        for line in content[start..].split_inclusive('\n') {
            let body = line.strip_suffix('\n').unwrap_or(line);
            let trimmed = body.trim_start();
            if let Some((marker, min_len)) = fenced_marker {
                if let Some((close_marker, close_len)) = parse_fence_marker(trimmed)
                    && close_marker == marker
                    && close_len >= min_len
                {
                    fenced_marker = None;
                }
            } else if let Some(marker) = parse_fence_marker(trimmed) {
                fenced_marker = Some(marker);
            } else {
                let level = trimmed.chars().take_while(|ch| *ch == '#').count();
                if (1..=6).contains(&level)
                    && trimmed[level..]
                        .chars()
                        .next()
                        .is_some_and(char::is_whitespace)
                {
                    headings.push(SectionHeading {
                        offset,
                        level,
                        line: trimmed.trim_end(),
                    });
                }
            }
            offset += line.len();
        }
        Self {
            content_len: content.len(),
            headings,
        }
    }

    pub(crate) fn headings(&self) -> &[SectionHeading<'a>] {
        &self.headings
    }

    /// Byte range of the section heading `index` opens: its heading line
    /// through everything before the next heading of the same or a higher
    /// level, or the end of the note. Subsections are inside it.
    pub(crate) fn span(&self, index: usize) -> Range<usize> {
        let heading = self.headings[index];
        let end = self.headings[index + 1..]
            .iter()
            .find(|candidate| candidate.level <= heading.level)
            .map_or(self.content_len, |next| next.offset);
        heading.offset..end
    }

    /// Each heading's path: the text of every heading whose section holds it,
    /// outermost first, then its own, joined with ` > `. The same form a
    /// search hit's `heading_path` has. A heading with no text adds nothing
    /// to a path, its own included, as the chunker that builds a hit's path
    /// passes over it.
    pub(crate) fn heading_paths(&self) -> Vec<String> {
        let mut open: Vec<SectionHeading<'a>> = Vec::new();
        self.headings
            .iter()
            .map(|heading| {
                if !heading.text().is_empty() {
                    while open.last().is_some_and(|last| last.level >= heading.level) {
                        open.pop();
                    }
                    open.push(*heading);
                }
                open.iter()
                    .map(SectionHeading::text)
                    .collect::<Vec<_>>()
                    .join(" > ")
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sections_of(content: &str) -> Vec<&str> {
        let sections = NoteSections::scan(content);
        (0..sections.headings().len())
            .map(|index| &content[sections.span(index)])
            .collect()
    }

    #[test]
    fn a_section_runs_to_the_next_heading_of_the_same_or_a_higher_level() {
        let content = "intro\n# A\na\n## B\nb\n### C\nc\n## D\nd\n# E\ne";
        assert_eq!(
            sections_of(content),
            vec![
                "# A\na\n## B\nb\n### C\nc\n## D\nd\n",
                "## B\nb\n### C\nc\n",
                "### C\nc\n",
                "## D\nd\n",
                "# E\ne",
            ]
        );
    }

    #[test]
    fn a_hash_line_inside_a_fenced_code_block_is_no_heading_and_ends_no_section() {
        let content = "# A\n```sh\n# not a heading\n```\n~~~\n## nor this\n~~~\ntail\n# B\n";
        let sections = NoteSections::scan(content);
        let lines: Vec<&str> = sections.headings().iter().map(|h| h.line).collect();
        assert_eq!(lines, vec!["# A", "# B"]);
        assert_eq!(
            &content[sections.span(0)],
            "# A\n```sh\n# not a heading\n```\n~~~\n## nor this\n~~~\ntail\n"
        );
    }

    #[test]
    fn a_heading_needs_one_to_six_hashes_and_a_space() {
        let content = "#tag\n####### seven\n###### six\n  ## indented  \r\n";
        let sections = NoteSections::scan(content);
        let found: Vec<(usize, &str, &str)> = sections
            .headings()
            .iter()
            .map(|h| (h.level, h.line, h.text()))
            .collect();
        assert_eq!(
            found,
            vec![(6, "###### six", "six"), (2, "## indented", "indented")]
        );
    }

    #[test]
    fn heading_paths_chain_the_enclosing_headings() {
        let content = "### Deep first\n# A\n### Skips a level\n## B\n#### C\n# D\n";
        assert_eq!(
            NoteSections::scan(content).heading_paths(),
            vec![
                "Deep first",
                "A",
                "A > Skips a level",
                "A > B",
                "A > B > C",
                "D",
            ]
        );
    }

    #[test]
    fn a_heading_with_no_text_adds_nothing_to_a_path() {
        let content = "# A\n## B\n## \n### C\n";
        assert_eq!(
            NoteSections::scan(content).heading_paths(),
            vec!["A", "A > B", "A > B", "A > B > C"]
        );
    }

    #[test]
    fn scanning_from_an_offset_skips_what_comes_before_and_keeps_whole_note_offsets() {
        let content = "---\n# a YAML comment\n---\n# Real\nbody\n";
        let start = content.find("# Real").unwrap();
        let sections = NoteSections::scan_from(content, start);
        assert_eq!(sections.headings().len(), 1);
        assert_eq!(sections.span(0), start..content.len());
        // The whole-content scan the write layer runs still sees both.
        assert_eq!(NoteSections::scan(content).headings().len(), 2);
    }
}
