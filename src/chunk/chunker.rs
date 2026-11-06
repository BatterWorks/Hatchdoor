use super::normalize::{
    Normalized, extract_frontmatter_metadata, restore_fences, strip_code_fences, strip_frontmatter,
};
use crate::embed::Embedder;
use crate::vault::NoteSections;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub ordinal: usize,
    pub heading_path: Option<String>,
    /// The text that is embedded and hashed: fence marker lines removed, code
    /// kept. ADR-16 locks it, so nothing a reader needs may be added here.
    pub content: String,
    /// The text the cache stores and a search hit returns: `content` with a
    /// fence marker line back around each run of code, so a reader can tell
    /// code from prose (#532). Equal to `content` when the chunk has no code.
    pub stored_content: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub content_hash: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct ChunkOptions {
    pub max_tokens: usize,
    pub overlap_tokens: usize,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        Self {
            max_tokens: 800,
            overlap_tokens: 50,
        }
    }
}

#[allow(dead_code)]
pub struct NoteChunking {
    pub chunks: Vec<Chunk>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
}

#[allow(dead_code)]
pub fn chunk_note(raw_content: &str, embedder: &dyn Embedder, opts: ChunkOptions) -> NoteChunking {
    use text_splitter::{ChunkConfig, ChunkSizer, MarkdownSplitter};

    struct EmbedderSizer<'a>(&'a dyn Embedder);

    impl ChunkSizer for EmbedderSizer<'_> {
        fn size(&self, chunk: &str) -> usize {
            self.0
                .token_count(chunk, false)
                .expect("embedder tokenizer must be able to count chunk tokens")
        }
    }

    let metadata = extract_frontmatter_metadata(raw_content);
    let body = strip_frontmatter(raw_content);
    let normalized = strip_code_fences(body);
    let headings = ChunkHeadings::of(raw_content, body, &normalized);

    if normalized.text.trim().is_empty() {
        return NoteChunking {
            chunks: Vec::new(),
            tags: metadata.tags,
            aliases: metadata.aliases,
        };
    }

    let config = ChunkConfig::new(opts.max_tokens)
        .with_sizer(EmbedderSizer(embedder))
        .with_overlap(opts.overlap_tokens)
        .expect("overlap must be < max_tokens");
    let splitter = MarkdownSplitter::new(config);

    let mut chunks = Vec::new();
    for (ordinal, (byte_start, piece)) in splitter.chunk_indices(&normalized.text).enumerate() {
        let byte_end = byte_start + piece.len();
        // Scan up to the start of the first non-heading content within this chunk
        // so that headings at the very beginning of a chunk are captured.
        let heading_scan_end = headings.content_start(piece, byte_start);
        let heading_path = headings.path_before(heading_scan_end);
        let content = piece.to_string();
        let stored_content = restore_fences(&normalized, byte_start, byte_end);
        let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        chunks.push(Chunk {
            ordinal,
            heading_path,
            content,
            stored_content,
            byte_start,
            byte_end,
            content_hash,
        });
    }

    NoteChunking {
        chunks,
        tags: metadata.tags,
        aliases: metadata.aliases,
    }
}

/// The headings a chunk's `heading_path` may name: the note's Sections as
/// `vault::sections` reads them, which is what `get_note_outline` lists and
/// `get_note_section` resolves, placed in the normalized text. Only levels one
/// to three shape a chunk's path (ADR-16).
struct ChunkHeadings {
    /// Offset of the heading's line in `Normalized::text`, and its path.
    /// Ascending by offset.
    headings: Vec<(usize, String)>,
}

impl ChunkHeadings {
    fn of(raw_content: &str, body: &str, normalized: &Normalized) -> Self {
        // `body` is a suffix of `raw_content`, so a heading's offset in the
        // body is its offset in the note less the frontmatter's length.
        let body_offset = raw_content.len() - body.len();
        let sections = NoteSections::scan(raw_content);
        let headings = sections
            .headings()
            .iter()
            .zip(sections.heading_paths())
            .filter(|(heading, _)| {
                heading.offset >= body_offset && heading.level <= 3 && !heading.text().is_empty()
            })
            .map(|(heading, path)| (normalized.text_offset(heading.offset - body_offset), path))
            .collect();
        Self { headings }
    }

    fn is_heading_at(&self, offset: usize) -> bool {
        self.headings
            .binary_search_by_key(&offset, |(start, _)| *start)
            .is_ok()
    }

    /// The offset of the first line in `piece` that is not a heading, where
    /// `piece` starts at `piece_start` in the normalized text, so a heading
    /// that opens a chunk counts towards its path.
    fn content_start(&self, piece: &str, piece_start: usize) -> usize {
        let mut offset = piece_start;
        for line in piece.split_inclusive('\n') {
            if !self.is_heading_at(offset) {
                break;
            }
            offset += line.len();
        }
        offset
    }

    /// The path of the last heading whose line starts before `offset`.
    fn path_before(&self, offset: usize) -> Option<String> {
        let count = self.headings.partition_point(|(start, _)| *start < offset);
        count
            .checked_sub(1)
            .map(|last| self.headings[last].1.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::{Embedder, StubEmbedder};

    fn chunk(content: &str, opts: ChunkOptions) -> NoteChunking {
        let embedder = StubEmbedder::new(384);
        chunk_note(content, &embedder, opts)
    }

    #[test]
    fn empty_input_produces_no_chunks() {
        let result = chunk("", ChunkOptions::default());
        assert!(result.chunks.is_empty());
    }

    #[test]
    fn small_single_section_produces_one_chunk() {
        let content = "# Heading\n\nA short paragraph.";
        let result = chunk(content, ChunkOptions::default());
        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].ordinal, 0);
        assert!(result.chunks[0].content.contains("short paragraph"));
    }

    #[test]
    fn chunks_have_deterministic_blake3_hashes() {
        let content = "# A\n\nbody";
        let a = chunk(content, ChunkOptions::default());
        let b = chunk(content, ChunkOptions::default());
        assert_eq!(a.chunks, b.chunks);
        assert_eq!(a.chunks[0].content_hash.len(), 64);
    }

    #[test]
    fn ordinals_are_sequential_from_zero() {
        let content = "# A\nfirst\n\n# B\nsecond\n\n# C\nthird";
        let result = chunk(
            content,
            ChunkOptions {
                max_tokens: 5,
                overlap_tokens: 0,
            },
        );
        for (i, chunk) in result.chunks.iter().enumerate() {
            assert_eq!(chunk.ordinal, i);
        }
        assert!(result.chunks.len() >= 3);
    }

    #[test]
    fn heading_path_reflects_nested_headings() {
        let content = "# Top\n\n## Sub\n\ndeep body";
        let result = chunk(content, ChunkOptions::default());
        let last = result.chunks.last().expect("chunk");
        let path = last.heading_path.as_deref().unwrap_or("");
        assert!(path.contains("Top") || path.contains("Sub"));
    }

    #[test]
    fn frontmatter_is_stripped_before_chunking() {
        let content = "---\ntags: [x, y]\n---\n\n# A\n\nbody";
        let result = chunk(content, ChunkOptions::default());
        assert!(
            result
                .chunks
                .iter()
                .all(|c| !c.content.contains("tags: [x, y]"))
        );
        assert_eq!(result.tags, vec!["x", "y"]);
    }

    #[test]
    fn code_fences_are_stripped_but_code_contents_remain() {
        let content = "# A\n\n```rust\nfn foo() {}\n```\n";
        let result = chunk(content, ChunkOptions::default());
        let joined: String = result.chunks.iter().map(|c| c.content.clone()).collect();
        assert!(joined.contains("fn foo()"));
        assert!(!joined.contains("```"));
    }

    #[test]
    fn stored_content_delimits_code_and_leaves_the_embedded_text_alone() {
        let content = "# A\n\nprose\n\n```mermaid\ngraph LR; A --> B\n```\n\nmore prose";
        let result = chunk(content, ChunkOptions::default());
        assert_eq!(result.chunks.len(), 1);
        let only = &result.chunks[0];
        // Byte for byte what the chunker produced before #532.
        assert_eq!(
            only.content,
            "# A\n\nprose\n\ngraph LR; A --> B\n\nmore prose"
        );
        assert_eq!(
            only.content_hash,
            blake3::hash(only.content.as_bytes()).to_hex().to_string()
        );
        assert_eq!(
            only.stored_content,
            "# A\n\nprose\n\n```mermaid\ngraph LR; A --> B\n```\n\nmore prose"
        );
    }

    #[test]
    fn a_block_split_across_chunks_is_whole_in_each_stored_content() {
        let code = "let value = compute(input);\n".repeat(40);
        let content = format!("# A\n\nintro\n\n```rust\n{code}```\n\noutro");
        let opts = ChunkOptions {
            max_tokens: 30,
            overlap_tokens: 0,
        };
        let result = chunk(&content, opts);
        let with_code: Vec<&Chunk> = result
            .chunks
            .iter()
            .filter(|c| c.content.contains("compute"))
            .collect();
        assert!(with_code.len() >= 2, "expected the block to be split");
        for c in with_code {
            assert_eq!(c.stored_content.matches("```").count(), 2, "{c:?}");
            assert!(c.stored_content.contains("```rust\n"), "{c:?}");
            assert!(!c.content.contains("```"), "{c:?}");
        }
        let prose_only = result
            .chunks
            .iter()
            .find(|c| !c.content.contains("compute"))
            .expect("a chunk without code");
        assert_eq!(prose_only.stored_content, prose_only.content);
    }

    #[test]
    fn shebang_and_comments_inside_code_fences_are_not_headings() {
        // A `#!/...` shebang and a `# comment` inside a fenced block must not be
        // parsed as ATX headings and pollute the heading path of later chunks.
        let content = "\
# Real Title

```bash
#!/usr/bin/env bash
# not a heading
echo one
echo two
```

## Ownership handling

body content that lands in a separate later chunk";
        let opts = ChunkOptions {
            max_tokens: 8,
            overlap_tokens: 0,
        };
        let result = chunk(content, opts);
        assert!(result.chunks.len() >= 2, "expected multiple chunks");
        for c in &result.chunks {
            let path = c.heading_path.as_deref().unwrap_or("");
            assert!(
                !path.contains("/usr/bin/env"),
                "fenced shebang leaked into heading path: {path:?}"
            );
            assert!(
                !path.contains("not a heading"),
                "fenced comment leaked into heading path: {path:?}"
            );
        }
        // The real heading after the fence is still derived correctly.
        let last = result.chunks.last().expect("chunk");
        let path = last.heading_path.as_deref().unwrap_or("");
        assert!(
            path.contains("Ownership handling"),
            "real heading lost after fence: {path:?}"
        );
    }

    /// Every heading path the note's chunks carry.
    fn heading_paths(content: &str, opts: ChunkOptions) -> Vec<Option<String>> {
        chunk(content, opts)
            .chunks
            .into_iter()
            .map(|c| c.heading_path)
            .collect()
    }

    #[test]
    fn a_tag_line_is_no_heading() {
        let paths = heading_paths(
            "#meeting #acme\n\nWe agreed on the plan.\n",
            ChunkOptions::default(),
        );
        assert_eq!(paths, vec![None]);
    }

    #[test]
    fn a_hash_line_in_a_tilde_fence_is_no_heading() {
        let filler = "Setup notes keep going here with more words. ".repeat(120);
        let content = format!("# Setup\n\n~~~bash\n# install the deps\nnpm ci\n~~~\n\n{filler}\n");
        let opts = ChunkOptions {
            max_tokens: 200,
            overlap_tokens: 0,
        };
        let paths = heading_paths(&content, opts);
        assert!(paths.len() > 1, "expected several chunks");
        assert!(
            paths.iter().all(|p| p.as_deref() == Some("Setup")),
            "{paths:?}"
        );
        let chunks = chunk(&content, opts).chunks;
        assert!(
            chunks.iter().any(|c| c
                .stored_content
                .contains("~~~bash\n# install the deps\nnpm ci\n~~~")),
            "{chunks:?}"
        );
        assert!(chunks.iter().all(|c| !c.content.contains("~~~")));
    }

    #[test]
    fn a_hash_line_in_a_nested_fence_is_no_heading() {
        let content = "# Showcase\n\n````markdown\n```js\n# not a heading either\nconsole.log(1)\n```\n````\n\nAfter.\n";
        let result = chunk(content, ChunkOptions::default());
        for c in &result.chunks {
            assert_eq!(c.heading_path.as_deref(), Some("Showcase"));
        }
        let stored: String = result
            .chunks
            .iter()
            .map(|c| c.stored_content.as_str())
            .collect();
        assert!(
            stored
                .contains("````markdown\n```js\n# not a heading either\nconsole.log(1)\n```\n````"),
            "{stored}"
        );
    }

    #[test]
    fn an_indented_code_line_is_no_heading() {
        let content = "# Real\n\nText.\n\n    # indented code\n\nMore.\n";
        let paths = heading_paths(content, ChunkOptions::default());
        assert_eq!(paths, vec![Some("Real".to_string())]);
    }

    #[test]
    fn heading_paths_match_the_note_sections_rule() {
        // Every path a chunk carries is a path the outline lists.
        let content = "---\ntags: [a]\n---\n# A\n\ntext\n\n#### Deep\n\nmore\n\n## B\n\n```sh\n# no\n```\n\n### C\n\nend\n";
        let opts = ChunkOptions {
            max_tokens: 8,
            overlap_tokens: 0,
        };
        let listed = crate::vault::NoteSections::scan(content).heading_paths();
        for path in heading_paths(content, opts).into_iter().flatten() {
            assert!(listed.contains(&path), "{path:?} not in {listed:?}");
        }
    }

    #[test]
    fn wikilinks_are_preserved_literally() {
        let content = "# A\n\nsee [[Other Note]] for context";
        let result = chunk(content, ChunkOptions::default());
        let joined: String = result.chunks.iter().map(|c| c.content.clone()).collect();
        assert!(joined.contains("[[Other Note]]"));
    }

    #[test]
    fn oversized_section_is_split_under_max_tokens() {
        let big = "para. ".repeat(2_000);
        let content = format!("# A\n\n{big}");
        let opts = ChunkOptions {
            max_tokens: 50,
            overlap_tokens: 5,
        };
        let embedder = StubEmbedder::new(384);
        let result = chunk_note(&content, &embedder, opts);
        for chunk in &result.chunks {
            assert!(
                embedder
                    .token_count(chunk.content.as_str(), false)
                    .expect("encode")
                    <= opts.max_tokens + opts.overlap_tokens
            );
        }
    }
}
