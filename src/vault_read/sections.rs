//! Partial reads of one Note (#502): its outline, and whole sections picked
//! by heading. Both go by the section rule `replace_section` writes by
//! (`vault::NoteSections`), over the Note's body. The frontmatter block and
//! the text before the first heading are sized by the outline and are not
//! sections.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::vault::NoteSections;
use crate::vault_registry::VaultId;

use super::text_match::body_start;

/// The most headings one section read may ask for.
pub const MAX_SECTION_HEADINGS: usize = 10;

/// `VaultReadCore::note_outline`'s answer: where a Note's bytes are, with no
/// body text. `frontmatter_bytes`, `opening_text_bytes` and the `size_bytes`
/// of the headings that sit inside no other heading's section add up to
/// `size_bytes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct NoteOutline {
    pub vault_id: VaultId,
    pub slug: String,
    pub relative_path: String,
    /// The whole file's hash, the same string an exact Note read reports.
    pub content_hash: String,
    /// The whole file, in bytes.
    pub size_bytes: usize,
    /// The leading frontmatter block with its delimiter lines; `0` without one.
    pub frontmatter_bytes: usize,
    /// The text between the frontmatter block and the first heading.
    pub opening_text_bytes: usize,
    /// Every heading in document order. Empty for a Note with none.
    pub headings: Vec<OutlineHeading>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct OutlineHeading {
    /// The heading without its `#` characters.
    pub text: String,
    /// 1 to 6.
    pub level: u8,
    /// The headings above this one and its own text, joined with ` > `.
    pub heading_path: String,
    /// The byte length of this heading's section, subsections included: what
    /// a section read returns for it.
    pub size_bytes: usize,
}

/// `VaultReadCore::note_sections`'s answer: one entry per requested string,
/// in the order asked, from a single read of the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct NoteSectionsResponse {
    pub vault_id: VaultId,
    pub slug: String,
    pub relative_path: String,
    /// The whole file's hash, the same string an exact Note read reports.
    pub content_hash: String,
    pub sections: Vec<NoteSectionEntry>,
}

/// What one requested string came to: the section it selected, or why it
/// selected none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(untagged)]
pub enum NoteSectionEntry {
    Found(NoteSection),
    Missed(NoteSectionMiss),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct NoteSection {
    /// The string as the caller sent it.
    pub requested: String,
    /// The heading path it resolved to.
    pub heading_path: String,
    /// 1 to 6.
    pub level: u8,
    /// The heading line and everything under it up to the next heading of
    /// the same or a higher level, byte for byte as the file holds it.
    pub section: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct NoteSectionMiss {
    /// The string as the caller sent it.
    pub requested: String,
    pub error: NoteSectionError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub struct NoteSectionError {
    pub code: NoteSectionErrorCode,
    pub message: String,
    /// For `heading_ambiguous`, the heading path of each heading that
    /// matched; empty otherwise.
    pub matches: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteSectionErrorCode {
    HeadingNotFound,
    HeadingAmbiguous,
}

/// The requested strings as they will be matched: trimmed, as
/// `replace_section` trims its heading. `Err` is the sentence refusing the
/// whole request.
pub(super) fn validate_headings(headings: &[String]) -> Result<(), String> {
    if headings.is_empty() {
        return Err("headings must name at least one heading".to_string());
    }
    if headings.len() > MAX_SECTION_HEADINGS {
        return Err(format!(
            "headings names {} headings; one call reads at most {MAX_SECTION_HEADINGS}",
            headings.len()
        ));
    }
    if headings.iter().any(|heading| heading.trim().is_empty()) {
        return Err("headings must not contain an empty string".to_string());
    }
    Ok(())
}

/// A Note's body headings, each with its path.
struct Outline<'a> {
    content: &'a str,
    body_start: usize,
    sections: NoteSections<'a>,
    paths: Vec<String>,
}

impl<'a> Outline<'a> {
    fn of(content: &'a str) -> Self {
        let body_start = body_start(content);
        let sections = NoteSections::scan_from(content, body_start);
        let paths = sections.heading_paths();
        Self {
            content,
            body_start,
            sections,
            paths,
        }
    }

    fn level(&self, index: usize) -> u8 {
        // A heading has at most six `#` characters.
        self.sections.headings()[index].level as u8
    }

    /// The one heading `requested` selects: by its text when exactly one
    /// heading has that text, and otherwise by its heading path.
    fn select(&self, requested: &str) -> Result<usize, NoteSectionError> {
        let headings = self.sections.headings();
        let by_text: Vec<usize> = (0..headings.len())
            .filter(|index| headings[*index].text() == requested)
            .collect();
        if let [index] = by_text.as_slice() {
            return Ok(*index);
        }
        let by_path: Vec<usize> = (0..headings.len())
            .filter(|index| self.paths[*index] == requested)
            .collect();
        if let [index] = by_path.as_slice() {
            return Ok(*index);
        }
        // Every heading the string could mean, by either reading, in
        // document order.
        let mut ambiguous = by_text;
        ambiguous.extend(by_path);
        ambiguous.sort_unstable();
        ambiguous.dedup();
        if ambiguous.is_empty() {
            return Err(NoteSectionError {
                code: NoteSectionErrorCode::HeadingNotFound,
                message: format!(
                    "No heading has the text or the heading path '{requested}'. Give a heading's text without its '#' characters, or its heading path."
                ),
                matches: Vec::new(),
            });
        }
        let matches: Vec<String> = ambiguous
            .iter()
            .map(|index| self.paths[*index].clone())
            .collect();
        let distinct = matches.iter().collect::<std::collections::BTreeSet<_>>();
        let message = if distinct.len() == matches.len() {
            format!(
                "'{requested}' matches {} headings. Ask for one by its heading path.",
                matches.len()
            )
        } else {
            format!(
                "'{requested}' matches {} headings, and some share one heading path, so no request can tell those apart.",
                matches.len()
            )
        };
        Err(NoteSectionError {
            code: NoteSectionErrorCode::HeadingAmbiguous,
            message,
            matches,
        })
    }
}

pub(super) fn note_outline(
    vault_id: VaultId,
    entry: &crate::vault::NoteEntry,
    content: &str,
) -> NoteOutline {
    let outline = Outline::of(content);
    let headings = outline.sections.headings();
    let first_heading = headings.first().map_or(content.len(), |h| h.offset);
    NoteOutline {
        vault_id,
        slug: entry.slug.clone(),
        relative_path: entry.relative_path.clone(),
        content_hash: crate::cache::parse::content_hash(content),
        size_bytes: content.len(),
        frontmatter_bytes: outline.body_start,
        opening_text_bytes: first_heading - outline.body_start,
        headings: headings
            .iter()
            .enumerate()
            .map(|(index, heading)| OutlineHeading {
                text: heading.text().to_string(),
                level: outline.level(index),
                heading_path: outline.paths[index].clone(),
                size_bytes: outline.sections.span(index).len(),
            })
            .collect(),
    }
}

/// `headings` must already have passed [`validate_headings`].
pub(super) fn note_sections(
    vault_id: VaultId,
    entry: &crate::vault::NoteEntry,
    content: &str,
    headings: &[String],
) -> NoteSectionsResponse {
    let outline = Outline::of(content);
    NoteSectionsResponse {
        vault_id,
        slug: entry.slug.clone(),
        relative_path: entry.relative_path.clone(),
        content_hash: crate::cache::parse::content_hash(content),
        sections: headings
            .iter()
            .map(|requested| match outline.select(requested.trim()) {
                Ok(index) => NoteSectionEntry::Found(NoteSection {
                    requested: requested.clone(),
                    heading_path: outline.paths[index].clone(),
                    level: outline.level(index),
                    section: outline.content[outline.sections.span(index)].to_string(),
                }),
                Err(error) => NoteSectionEntry::Missed(NoteSectionMiss {
                    requested: requested.clone(),
                    error,
                }),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::vault::NoteEntry;

    const NESTED: &str = "---\ntitle: Rules\n# a YAML comment\n---\nOpening text.\n\n# Filing\nWhere notes go.\n## Notes\nFiling notes.\n### Deep\nDeep text.\n#### Deeper\nDeeper text.\n# Tags\n## Notes\nTag notes.\n```md\n# Not a heading\n```\nAfter the fence.\n# Ünïcode 日本語\nlast";

    fn entry() -> NoteEntry {
        NoteEntry {
            title: "Rules".to_string(),
            slug: "rules".to_string(),
            path: PathBuf::from("/nowhere/Rules.md"),
            relative_path: "Rules".to_string(),
            layer: None,
        }
    }

    fn vault_id() -> VaultId {
        VaultId::generate().expect("Vault id")
    }

    fn read(content: &str, headings: &[&str]) -> Vec<NoteSectionEntry> {
        let headings: Vec<String> = headings.iter().map(|h| (*h).to_string()).collect();
        validate_headings(&headings).expect("valid request");
        note_sections(vault_id(), &entry(), content, &headings).sections
    }

    fn found(entry: &NoteSectionEntry) -> &NoteSection {
        match entry {
            NoteSectionEntry::Found(section) => section,
            NoteSectionEntry::Missed(miss) => panic!("expected a section, got {miss:?}"),
        }
    }

    fn missed(entry: &NoteSectionEntry) -> &NoteSectionError {
        match entry {
            NoteSectionEntry::Missed(miss) => &miss.error,
            NoteSectionEntry::Found(section) => panic!("expected a miss, got {section:?}"),
        }
    }

    #[test]
    fn the_outline_lists_every_body_heading_with_its_path_and_level() {
        let outline = note_outline(vault_id(), &entry(), NESTED);
        let listed: Vec<(&str, u8, &str)> = outline
            .headings
            .iter()
            .map(|h| (h.text.as_str(), h.level, h.heading_path.as_str()))
            .collect();
        assert_eq!(
            listed,
            vec![
                ("Filing", 1, "Filing"),
                ("Notes", 2, "Filing > Notes"),
                ("Deep", 3, "Filing > Notes > Deep"),
                ("Deeper", 4, "Filing > Notes > Deep > Deeper"),
                ("Tags", 1, "Tags"),
                ("Notes", 2, "Tags > Notes"),
                ("Ünïcode 日本語", 1, "Ünïcode 日本語"),
            ]
        );
        assert_eq!(outline.slug, "rules");
        assert_eq!(outline.relative_path, "Rules");
        assert_eq!(
            outline.content_hash,
            crate::cache::parse::content_hash(NESTED)
        );
    }

    #[test]
    fn each_outline_size_is_the_byte_length_of_the_section_its_path_returns() {
        let outline = note_outline(vault_id(), &entry(), NESTED);
        for heading in &outline.headings {
            let entries = read(NESTED, &[&heading.heading_path]);
            let section = found(&entries[0]);
            assert_eq!(
                section.section.len(),
                heading.size_bytes,
                "{}",
                heading.heading_path
            );
            assert_eq!(section.heading_path, heading.heading_path);
            assert_eq!(section.level, heading.level);
        }
    }

    #[test]
    fn frontmatter_opening_text_and_top_level_sections_add_up_to_the_note() {
        for content in [
            NESTED,
            "## Starts low\na\n# Then high\nb\n### Then lower\nc\n",
            "no frontmatter\n# A\n",
            "---\nunterminated: frontmatter\n# A\n",
            "",
        ] {
            let outline = note_outline(vault_id(), &entry(), content);
            let mut open: Vec<u8> = Vec::new();
            let mut top_level = 0;
            for heading in &outline.headings {
                while open.last().is_some_and(|level| *level >= heading.level) {
                    open.pop();
                }
                if open.is_empty() {
                    top_level += heading.size_bytes;
                }
                open.push(heading.level);
            }
            assert_eq!(
                outline.frontmatter_bytes + outline.opening_text_bytes + top_level,
                outline.size_bytes,
                "{content:?}"
            );
            assert_eq!(outline.size_bytes, content.len());
        }
        let outline = note_outline(vault_id(), &entry(), NESTED);
        assert_eq!(
            outline.frontmatter_bytes,
            "---\ntitle: Rules\n# a YAML comment\n---\n".len()
        );
        assert_eq!(outline.opening_text_bytes, "Opening text.\n\n".len());
    }

    #[test]
    fn a_note_with_no_headings_has_an_empty_outline_and_correct_sizes() {
        let content = "---\ntags: [a]\n---\nJust prose.\n```\n# fenced\n```\n";
        let outline = note_outline(vault_id(), &entry(), content);
        assert!(outline.headings.is_empty());
        assert_eq!(outline.size_bytes, content.len());
        assert_eq!(outline.frontmatter_bytes, "---\ntags: [a]\n---\n".len());
        assert_eq!(
            outline.opening_text_bytes,
            content.len() - outline.frontmatter_bytes
        );

        let empty = note_outline(vault_id(), &entry(), "");
        assert!(empty.headings.is_empty());
        assert_eq!(
            (
                empty.size_bytes,
                empty.frontmatter_bytes,
                empty.opening_text_bytes
            ),
            (0, 0, 0)
        );
    }

    #[test]
    fn a_hash_line_in_a_fence_or_in_the_frontmatter_is_in_no_outline_and_ends_no_section() {
        let outline = note_outline(vault_id(), &entry(), NESTED);
        assert!(
            outline
                .headings
                .iter()
                .all(|h| h.text != "Not a heading" && h.text != "a YAML comment")
        );
        let entries = read(NESTED, &["Tags > Notes"]);
        assert_eq!(
            found(&entries[0]).section,
            "## Notes\nTag notes.\n```md\n# Not a heading\n```\nAfter the fence.\n"
        );
        for hidden in ["Not a heading", "a YAML comment"] {
            let entries = read(NESTED, &[hidden]);
            assert_eq!(
                missed(&entries[0]).code,
                NoteSectionErrorCode::HeadingNotFound
            );
        }
    }

    #[test]
    fn a_unique_heading_is_found_by_its_text_alone() {
        let entries = read(NESTED, &["Deep", "  Tags  "]);
        let deep = found(&entries[0]);
        assert_eq!(deep.heading_path, "Filing > Notes > Deep");
        assert_eq!(
            deep.section,
            "### Deep\nDeep text.\n#### Deeper\nDeeper text.\n"
        );
        // The request is trimmed, as `replace_section` trims its heading,
        // and echoed as sent.
        let tags = found(&entries[1]);
        assert_eq!(tags.requested, "  Tags  ");
        assert_eq!(tags.heading_path, "Tags");
        // The last section runs to the end of a file with no final newline.
        let entries = read(NESTED, &["Ünïcode 日本語"]);
        assert_eq!(found(&entries[0]).section, "# Ünïcode 日本語\nlast");
    }

    #[test]
    fn a_text_that_appears_twice_is_refused_with_both_paths_and_each_path_then_finds_one() {
        let entries = read(NESTED, &["Notes"]);
        let error = missed(&entries[0]);
        assert_eq!(error.code, NoteSectionErrorCode::HeadingAmbiguous);
        assert_eq!(error.matches, vec!["Filing > Notes", "Tags > Notes"]);

        let matches: Vec<&str> = error.matches.iter().map(String::as_str).collect();
        let entries = read(NESTED, &matches);
        assert!(
            found(&entries[0])
                .section
                .starts_with("## Notes\nFiling notes.\n")
        );
        assert!(
            found(&entries[1])
                .section
                .starts_with("## Notes\nTag notes.\n")
        );
    }

    #[test]
    fn a_repeated_text_whose_path_is_unique_resolves_by_that_path() {
        // "Log" is two headings by text, and the path "Log" names only the
        // top-level one.
        let content = "# Log\ntop\n# Week\n## Log\nnested\n";
        let entries = read(content, &["Log", "Week > Log"]);
        assert_eq!(found(&entries[0]).section, "# Log\ntop\n");
        assert_eq!(found(&entries[1]).section, "## Log\nnested\n");
    }

    #[test]
    fn two_headings_sharing_a_full_path_are_ambiguous_and_say_so() {
        let content = "# A\n## Same\none\n## Same\ntwo\n";
        let entries = read(content, &["Same", "A > Same"]);
        for entry in &entries {
            let error = missed(entry);
            assert_eq!(error.code, NoteSectionErrorCode::HeadingAmbiguous);
            assert_eq!(error.matches, vec!["A > Same", "A > Same"]);
            assert!(error.message.contains("share one heading path"));
        }
    }

    #[test]
    fn three_requests_with_one_missing_return_two_sections_and_one_error_in_order() {
        let entries = read(
            NESTED,
            &["Tags", "No such heading", "Filing > Notes > Deep"],
        );
        assert_eq!(entries.len(), 3);
        assert_eq!(found(&entries[0]).heading_path, "Tags");
        let error = missed(&entries[1]);
        assert_eq!(error.code, NoteSectionErrorCode::HeadingNotFound);
        assert!(error.matches.is_empty());
        assert_eq!(found(&entries[2]).heading_path, "Filing > Notes > Deep");
        let NoteSectionEntry::Missed(miss) = &entries[1] else {
            unreachable!()
        };
        assert_eq!(miss.requested, "No such heading");
    }

    #[test]
    fn matching_is_exact_on_case_and_never_reads_the_hash_spelling() {
        for request in ["tags", "# Tags", "Filing >Notes", "Filing"] {
            let entries = read(NESTED, &[request]);
            match request {
                "Filing" => assert_eq!(found(&entries[0]).level, 1),
                _ => assert_eq!(
                    missed(&entries[0]).code,
                    NoteSectionErrorCode::HeadingNotFound,
                    "{request}"
                ),
            }
        }
    }

    #[test]
    fn an_empty_list_an_empty_string_or_more_than_ten_headings_is_refused() {
        let strings = |items: &[&str]| -> Vec<String> {
            items.iter().map(|item| (*item).to_string()).collect()
        };
        assert!(validate_headings(&[]).is_err());
        assert!(validate_headings(&strings(&["Tags", ""])).is_err());
        assert!(validate_headings(&strings(&["   "])).is_err());
        assert!(validate_headings(&vec!["Tags".to_string(); 10]).is_ok());
        assert!(validate_headings(&vec!["Tags".to_string(); 11]).is_err());
    }

    #[test]
    fn no_reply_field_is_named_content() {
        let reply = note_sections(
            vault_id(),
            &entry(),
            NESTED,
            &["Tags".to_string(), "missing".to_string()],
        );
        let json = serde_json::to_string(&reply).expect("serialize");
        assert!(!json.contains("\"content\""), "{json}");
        let outline = serde_json::to_string(&note_outline(vault_id(), &entry(), NESTED)).unwrap();
        assert!(!outline.contains("\"content\""), "{outline}");
        // The outline carries no body text at all.
        assert!(!outline.contains("Where notes go"));
    }
    #[test]
    fn a_search_hits_heading_path_resolves_when_it_is_unique_in_the_note() {
        let content = "# Rules\nintro words\n## Filing\nfiling words\n### Inbox\ninbox words\n## Tags\ntag words\n";
        let embedder = crate::embed::StubEmbedder::new(384);
        let chunking = crate::chunk::chunk_note(
            content,
            &embedder,
            crate::chunk::ChunkOptions {
                max_tokens: 4,
                overlap_tokens: 0,
            },
        );
        let paths: std::collections::BTreeSet<String> = chunking
            .chunks
            .iter()
            .filter_map(|chunk| chunk.heading_path.clone())
            .collect();
        assert!(paths.contains("Rules > Filing > Inbox"), "{paths:?}");
        for path in &paths {
            let entries = read(content, &[path]);
            assert_eq!(&found(&entries[0]).heading_path, path);
        }
    }
    #[test]
    fn an_ambiguous_request_lists_every_heading_it_matched_by_text_or_by_path() {
        let content = "# Log\none\n# Log\ntwo\n## Log\nthree\n";
        let entries = read(content, &["Log"]);
        let error = missed(&entries[0]);
        assert_eq!(error.code, NoteSectionErrorCode::HeadingAmbiguous);
        assert_eq!(error.matches, vec!["Log", "Log", "Log > Log"]);
        // The nested one has a path of its own.
        let entries = read(content, &["Log > Log"]);
        assert_eq!(found(&entries[0]).section, "## Log\nthree\n");
    }
}
