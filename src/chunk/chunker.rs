use std::sync::Arc;

use tokenizers::Tokenizer;

use super::normalize::{extract_frontmatter_metadata, strip_code_fences, strip_frontmatter};

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Chunk {
    pub ordinal: usize,
    pub heading_path: Option<String>,
    pub content: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub content_hash: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct ChunkOptions {
    pub max_tokens: usize,
    pub overlap_tokens: usize,
}

impl Default for ChunkOptions {
    fn default() -> Self { Self { max_tokens: 800, overlap_tokens: 50 } }
}

#[allow(dead_code)]
pub(crate) struct NoteChunking {
    pub chunks: Vec<Chunk>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
}

#[allow(dead_code)]
pub(crate) fn chunk_note(
    raw_content: &str,
    tokenizer: Arc<Tokenizer>,
    opts: ChunkOptions,
) -> NoteChunking {
    use text_splitter::{ChunkConfig, MarkdownSplitter};

    let metadata = extract_frontmatter_metadata(raw_content);
    let body = strip_frontmatter(raw_content);
    let normalized = strip_code_fences(body);

    if normalized.trim().is_empty() {
        return NoteChunking {
            chunks: Vec::new(),
            tags: metadata.tags,
            aliases: metadata.aliases,
        };
    }

    let config = ChunkConfig::new(opts.max_tokens)
        .with_sizer((*tokenizer).clone())
        .with_overlap(opts.overlap_tokens)
        .expect("overlap must be < max_tokens");
    let splitter = MarkdownSplitter::new(config);

    let mut chunks = Vec::new();
    for (ordinal, (byte_start, piece)) in splitter.chunk_indices(&normalized).enumerate() {
        let byte_end = byte_start + piece.len();
        // Scan up to the start of the first non-heading content within this chunk
        // so that headings at the very beginning of a chunk are captured.
        let heading_scan_end = heading_content_start(piece, byte_start);
        let heading_path = derive_heading_path(&normalized, heading_scan_end);
        let content = piece.to_string();
        let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        chunks.push(Chunk { ordinal, heading_path, content, byte_start, byte_end, content_hash });
    }

    NoteChunking { chunks, tags: metadata.tags, aliases: metadata.aliases }
}

#[allow(dead_code)]
/// Returns the byte offset (relative to `content` start) of the first non-heading
/// line in `piece`, offset by `piece_start`. This allows headings that open a chunk
/// to be included in the heading path scan.
fn heading_content_start(piece: &str, piece_start: usize) -> usize {
    let mut offset = piece_start;
    for line in piece.lines() {
        let trimmed = line.trim_start();
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        if level > 0 && level <= 3 && !trimmed[level..].trim().is_empty() {
            // This is a heading line; advance past it (+ 1 for '\n')
            offset += line.len() + 1;
        } else {
            // First non-heading content; stop here
            break;
        }
    }
    offset
}

#[allow(dead_code)]
fn derive_heading_path(content: &str, byte_offset: usize) -> Option<String> {
    let prefix = &content[..byte_offset.min(content.len())];
    let mut stack: [Option<String>; 3] = [None, None, None];
    for line in prefix.lines() {
        let trimmed = line.trim_start();
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        if level == 0 || level > 3 { continue; }
        let text = trimmed[level..].trim();
        if text.is_empty() { continue; }
        stack[level - 1] = Some(text.to_string());
        for deeper in &mut stack[level..] { *deeper = None; }
    }
    let parts: Vec<String> = stack.iter().filter_map(Clone::clone).collect();
    if parts.is_empty() { None } else { Some(parts.join(" > ")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::{Embedder, StubEmbedder};

    fn stub_tokenizer() -> Arc<Tokenizer> {
        StubEmbedder::new(384).tokenizer()
    }

    #[test]
    fn empty_input_produces_no_chunks() {
        let result = chunk_note("", stub_tokenizer(), ChunkOptions::default());
        assert!(result.chunks.is_empty());
    }

    #[test]
    fn small_single_section_produces_one_chunk() {
        let content = "# Heading\n\nA short paragraph.";
        let result = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].ordinal, 0);
        assert!(result.chunks[0].content.contains("short paragraph"));
    }

    #[test]
    fn chunks_have_deterministic_blake3_hashes() {
        let content = "# A\n\nbody";
        let a = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        let b = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        assert_eq!(a.chunks, b.chunks);
        assert_eq!(a.chunks[0].content_hash.len(), 64);
    }

    #[test]
    fn ordinals_are_sequential_from_zero() {
        let content = "# A\nfirst\n\n# B\nsecond\n\n# C\nthird";
        let result = chunk_note(content, stub_tokenizer(), ChunkOptions { max_tokens: 5, overlap_tokens: 0 });
        for (i, chunk) in result.chunks.iter().enumerate() {
            assert_eq!(chunk.ordinal, i);
        }
        assert!(result.chunks.len() >= 3);
    }

    #[test]
    fn heading_path_reflects_nested_headings() {
        let content = "# Top\n\n## Sub\n\ndeep body";
        let result = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        let last = result.chunks.last().expect("chunk");
        let path = last.heading_path.as_deref().unwrap_or("");
        assert!(path.contains("Top") || path.contains("Sub"));
    }

    #[test]
    fn frontmatter_is_stripped_before_chunking() {
        let content = "---\ntags: [x, y]\n---\n\n# A\n\nbody";
        let result = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        assert!(result.chunks.iter().all(|c| !c.content.contains("tags: [x, y]")));
        assert_eq!(result.tags, vec!["x", "y"]);
    }

    #[test]
    fn code_fences_are_stripped_but_code_contents_remain() {
        let content = "# A\n\n```rust\nfn foo() {}\n```\n";
        let result = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        let joined: String = result.chunks.iter().map(|c| c.content.clone()).collect();
        assert!(joined.contains("fn foo()"));
        assert!(!joined.contains("```"));
    }

    #[test]
    fn wikilinks_are_preserved_literally() {
        let content = "# A\n\nsee [[Other Note]] for context";
        let result = chunk_note(content, stub_tokenizer(), ChunkOptions::default());
        let joined: String = result.chunks.iter().map(|c| c.content.clone()).collect();
        assert!(joined.contains("[[Other Note]]"));
    }

    #[test]
    fn oversized_section_is_split_under_max_tokens() {
        let big = "para. ".repeat(2_000);
        let content = format!("# A\n\n{big}");
        let opts = ChunkOptions { max_tokens: 50, overlap_tokens: 5 };
        let result = chunk_note(&content, stub_tokenizer(), opts);
        let tokenizer = stub_tokenizer();
        for chunk in &result.chunks {
            let encoding = tokenizer.encode(chunk.content.as_str(), false).expect("encode");
            assert!(encoding.get_ids().len() <= opts.max_tokens + opts.overlap_tokens);
        }
    }
}
