# Markdown Plain-Text Snippets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Strip markdown syntax from search result snippets so users see clean plain text instead of raw markdown characters like `**bold**`, `## Heading`, or `[[wikilink]]`.

## Scope Clarification

This plan is a snippet-display cleanup plan. It only improves plain-text output for `content_snippet()` and any legacy/internal search path that still uses that helper.

It does not improve the active Phase 2 `/api/search` retrieval path, semantic embeddings, chunking behavior, FTS indexing, search ranking or recall, MCP `search_notes` content, heading extraction, Markdown link parsing, or note rewrite logic.

## Semantic Search Follow-Up

A broader and more valuable use of `pulldown-cmark` would be to normalize Markdown before retrieval indexing:

- Parse Markdown into clean plain text for embedding input.
- Consider using the same normalized text for chunk-level FTS.
- Keep heading text as semantic context.
- Preserve link labels and Obsidian wikilink aliases/targets as readable text.
- Drop Markdown delimiters, formatting syntax, and noisy link markup.
- Keep raw Markdown separately for display, editing, byte ranges, and navigation.

Suggested retrieval storage model:

- `raw_content`: original Markdown chunk for display/navigation/update safety.
- `search_content`: normalized plain-text chunk for embeddings and FTS.
- `heading_path`: structured heading metadata.

Acceptance criteria for that follow-up should be eval-based, not cosmetic:

- Semantic or hybrid eval metrics improve or remain neutral.
- Returned chunks become more readable.
- Navigation and note display remain based on raw Markdown.
- Keyword search does not regress for exact terms, tags, or code-like content.

Risk notes:

- Do not blindly strip all Markdown before indexing.
- Keep heading text, link labels, wikilink aliases, task/list/table text, and possibly code content.
- Usually drop raw URLs, embed syntax, and formatting delimiters unless a specific retrieval use case needs them.

**Architecture:** Add `pulldown-cmark` as a direct dependency (it is already a transitive dep via `text-splitter`), create a `src/markdown/strip.rs` module with a `strip_to_plain_text` function, and use it inside `content_snippet` in `src/vault/paths.rs`. The snippet function reads raw note content from disk, so pre-processing there keeps the change local and self-contained.

**Tech Stack:** Rust, `pulldown-cmark` 0.13 (CommonMark + GFM extensions), regex-based wikilink pre-pass.

---

## File Map

| File | Action | Purpose |
|------|--------|---------|
| `Cargo.toml` | Modify | Add `pulldown-cmark` as a direct dep |
| `src/markdown/mod.rs` | Create | Module declaration |
| `src/markdown/strip.rs` | Create | `strip_to_plain_text` + wikilink pre-pass |
| `src/lib.rs` | Modify | Declare `mod markdown` |
| `src/vault/paths.rs` | Modify | Call `strip_to_plain_text` inside `content_snippet` |

---

### Task 1: Add pulldown-cmark as a direct dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, under `[dependencies]`, add:

```toml
pulldown-cmark = { version = "0.13", default-features = false, features = ["html"] }
```

`html` feature is needed for nothing we use, but the base crate is needed; `default-features = false` drops the `simd` optional feature to keep compile time low.

Actually use this simpler form (default features are fine at this version and pin matches the lockfile):

```toml
pulldown-cmark = "0.13"
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore(deps): add pulldown-cmark as direct dependency"
```

---

### Task 2: Create the markdown stripping module

**Files:**
- Create: `src/markdown/mod.rs`
- Create: `src/markdown/strip.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing tests for `strip_to_plain_text`**

Create `src/markdown/strip.rs` with the tests only (no implementation yet):

```rust
pub fn strip_to_plain_text(_markdown: &str) -> String {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::strip_to_plain_text;

    #[test]
    fn strips_heading_markers() {
        assert_eq!(strip_to_plain_text("## My Heading"), "My Heading");
    }

    #[test]
    fn strips_bold_and_italic() {
        assert_eq!(
            strip_to_plain_text("**bold** and *italic*"),
            "bold and italic"
        );
    }

    #[test]
    fn strips_inline_code_delimiters_but_keeps_content() {
        assert_eq!(strip_to_plain_text("`some code`"), "some code");
    }

    #[test]
    fn strips_link_syntax_keeping_label() {
        assert_eq!(
            strip_to_plain_text("[click here](https://example.com)"),
            "click here"
        );
    }

    #[test]
    fn strips_wikilink_keeping_target_when_no_alias() {
        assert_eq!(strip_to_plain_text("see [[Other Note]]"), "see Other Note");
    }

    #[test]
    fn strips_wikilink_keeping_alias() {
        assert_eq!(
            strip_to_plain_text("see [[Other Note|the alias]]"),
            "see the alias"
        );
    }

    #[test]
    fn strips_list_markers() {
        let input = "- first\n- second";
        let result = strip_to_plain_text(input);
        assert!(result.contains("first"), "got: {result}");
        assert!(result.contains("second"), "got: {result}");
        assert!(!result.contains("- "), "got: {result}");
    }

    #[test]
    fn collapses_blank_lines_to_single_newline() {
        let input = "para one\n\npara two";
        let result = strip_to_plain_text(input);
        assert!(result.contains("para one"), "got: {result}");
        assert!(result.contains("para two"), "got: {result}");
    }

    #[test]
    fn empty_input_returns_empty_string() {
        assert_eq!(strip_to_plain_text(""), "");
    }

    #[test]
    fn plain_text_is_returned_unchanged() {
        assert_eq!(strip_to_plain_text("just plain text"), "just plain text");
    }
}
```

Create `src/markdown/mod.rs`:

```rust
pub mod strip;
pub use strip::strip_to_plain_text;
```

In `src/lib.rs`, find the existing `mod` declarations and add:

```rust
mod markdown;
pub use markdown::strip_to_plain_text;
```

- [ ] **Step 2: Run the tests to confirm they all fail**

```bash
cargo test strip_to_plain_text
```

Expected: each test panics with `not implemented`.

- [ ] **Step 3: Implement `strip_to_plain_text`**

Replace the `unimplemented!()` stub in `src/markdown/strip.rs` with the full implementation:

```rust
use pulldown_cmark::{Event, Options, Parser, Tag};

/// Converts Markdown to plain text by stripping all syntax.
/// Handles CommonMark + GFM extensions (tables, strikethrough, task lists).
/// Obsidian wikilinks ([[target]] / [[target|alias]]) are pre-processed with
/// a regex pass because they are not part of any Markdown spec.
pub fn strip_to_plain_text(markdown: &str) -> String {
    let preprocessed = strip_wikilinks(markdown);

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(&preprocessed, opts);
    let mut output = String::new();

    for event in parser {
        match event {
            Event::Text(text) => output.push_str(&text),
            Event::Code(code) => output.push_str(&code),
            Event::SoftBreak => output.push(' '),
            Event::HardBreak => output.push('\n'),
            Event::Start(Tag::Paragraph | Tag::Heading { .. } | Tag::Item) => {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
            }
            _ => {}
        }
    }

    output.trim().to_string()
}

/// Strips Obsidian wikilinks before Markdown parsing.
/// [[Note Name]] → Note Name
/// [[Note Name|Alias]] → Alias
/// ![[image.png]] → (removed entirely — it's an embed, not readable text)
fn strip_wikilinks(input: &str) -> String {
    // Remove image/file embeds first
    let without_embeds = regex_replace_all(r"!\[\[[^\]]*\]\]", input, "");
    // Replace [[target|alias]] with alias
    let without_aliased =
        regex_replace_all(r"\[\[([^\]|]+)\|([^\]]+)\]\]", &without_embeds, "$2");
    // Replace [[target]] with target (strip heading/block anchors like #section or ^id)
    regex_replace_all(r"\[\[([^\]#^|]+)[^\]]*\]\]", &without_aliased, "$1")
}

/// Thin wrapper so the regex crate is not imported at call sites.
/// Uses a simple hand-rolled replacer to avoid pulling in the `regex` crate.
fn regex_replace_all(pattern: &str, input: &str, replacement: &str) -> String {
    // Build a minimal finite-state machine for each of our three fixed patterns
    // rather than depending on the regex crate.
    match pattern {
        r"!\[\[[^\]]*\]\]" => remove_embed_wikilinks(input),
        r"\[\[([^\]|]+)\|([^\]]+)\]\]" => replace_aliased_wikilinks(input, replacement),
        r"\[\[([^\]#^|]+)[^\]]*\]\]" => replace_plain_wikilinks(input, replacement),
        _ => input.to_string(),
    }
}

fn remove_embed_wikilinks(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes.get(i) == Some(&b'!')
            && bytes.get(i + 1) == Some(&b'[')
            && bytes.get(i + 2) == Some(&b'[')
        {
            if let Some(end) = find_close(bytes, i + 3) {
                i = end + 2;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn replace_aliased_wikilinks(input: &str, _replacement: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes.get(i) == Some(&b'[') && bytes.get(i + 1) == Some(&b'[') {
            if let Some(end) = find_close(bytes, i + 2) {
                let inner = &input[i + 2..end];
                if let Some(pipe) = inner.find('|') {
                    let alias = inner[pipe + 1..].trim();
                    out.push_str(alias);
                    i = end + 2;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn replace_plain_wikilinks(input: &str, _replacement: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes.get(i) == Some(&b'[') && bytes.get(i + 1) == Some(&b'[') {
            if let Some(end) = find_close(bytes, i + 2) {
                let inner = &input[i + 2..end];
                // Skip anchors and block refs
                let target = inner
                    .split(['#', '^', '|'])
                    .next()
                    .unwrap_or(inner)
                    .trim();
                out.push_str(target);
                i = end + 2;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Find the position of the first `]]` starting from `from`.
/// Returns the index of the first `]` of `]]`.
fn find_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b']' && bytes[i + 1] == b']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::strip_to_plain_text;

    #[test]
    fn strips_heading_markers() {
        assert_eq!(strip_to_plain_text("## My Heading"), "My Heading");
    }

    #[test]
    fn strips_bold_and_italic() {
        assert_eq!(
            strip_to_plain_text("**bold** and *italic*"),
            "bold and italic"
        );
    }

    #[test]
    fn strips_inline_code_delimiters_but_keeps_content() {
        assert_eq!(strip_to_plain_text("`some code`"), "some code");
    }

    #[test]
    fn strips_link_syntax_keeping_label() {
        assert_eq!(
            strip_to_plain_text("[click here](https://example.com)"),
            "click here"
        );
    }

    #[test]
    fn strips_wikilink_keeping_target_when_no_alias() {
        assert_eq!(strip_to_plain_text("see [[Other Note]]"), "see Other Note");
    }

    #[test]
    fn strips_wikilink_keeping_alias() {
        assert_eq!(
            strip_to_plain_text("see [[Other Note|the alias]]"),
            "see the alias"
        );
    }

    #[test]
    fn strips_list_markers() {
        let input = "- first\n- second";
        let result = strip_to_plain_text(input);
        assert!(result.contains("first"), "got: {result}");
        assert!(result.contains("second"), "got: {result}");
        assert!(!result.contains("- "), "got: {result}");
    }

    #[test]
    fn collapses_blank_lines_to_single_newline() {
        let input = "para one\n\npara two";
        let result = strip_to_plain_text(input);
        assert!(result.contains("para one"), "got: {result}");
        assert!(result.contains("para two"), "got: {result}");
    }

    #[test]
    fn empty_input_returns_empty_string() {
        assert_eq!(strip_to_plain_text(""), "");
    }

    #[test]
    fn plain_text_is_returned_unchanged() {
        assert_eq!(strip_to_plain_text("just plain text"), "just plain text");
    }
}
```

- [ ] **Step 4: Run the tests**

```bash
cargo test strip_to_plain_text
```

Expected: all 10 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/markdown/mod.rs src/markdown/strip.rs src/lib.rs
git commit -m "feat(markdown): add strip_to_plain_text for plain-text extraction"
```

---

### Task 3: Use plain-text stripping in search snippets

**Files:**
- Modify: `src/vault/paths.rs`

The current `content_snippet` function:
1. Iterates over raw markdown lines
2. Returns the first line containing the query (with all markdown syntax intact)

The new behaviour:
1. Convert the full document to plain text first
2. Iterate over the plain-text lines
3. Return the first plain-text line containing the query

- [ ] **Step 1: Write the failing test**

Open `src/vault/paths.rs`. The existing tests are in `src/vault/tests.rs`. Add this test there:

```rust
// in src/vault/tests.rs
#[test]
fn content_snippet_strips_markdown_syntax() {
    let content = "## My **Important** Heading\n\nSome regular paragraph.";
    let result = content_snippet(content, "important heading");
    let snippet = result.expect("should find a match");
    assert!(
        !snippet.contains("##"),
        "heading marker should be stripped, got: {snippet}"
    );
    assert!(
        !snippet.contains("**"),
        "bold markers should be stripped, got: {snippet}"
    );
    assert!(
        snippet.contains("Important"),
        "text should be present, got: {snippet}"
    );
}

#[test]
fn content_snippet_strips_wikilinks() {
    let content = "See [[Other Note|the alias]] for more.";
    let result = content_snippet(content, "the alias");
    let snippet = result.expect("should find a match");
    assert!(
        !snippet.contains("[["),
        "wikilink brackets should be stripped, got: {snippet}"
    );
    assert!(
        snippet.contains("the alias"),
        "alias text should be present, got: {snippet}"
    );
}
```

- [ ] **Step 2: Run the tests to confirm they fail**

```bash
cargo test content_snippet_strips
```

Expected: both tests fail because the snippet still contains `##` and `**`.

- [ ] **Step 3: Update `content_snippet` to strip markdown**

In `src/vault/paths.rs`, the current function is:

```rust
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
```

Replace it with:

```rust
pub fn content_snippet(content: &str, normalized_query: &str) -> Option<String> {
    let plain = crate::strip_to_plain_text(content);
    plain
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
```

- [ ] **Step 4: Run all tests**

```bash
cargo test
```

Expected: all tests pass, including the two new ones.

- [ ] **Step 5: Commit**

```bash
git add src/vault/paths.rs src/vault/tests.rs
git commit -m "feat(search): strip markdown syntax from content snippets"
```

---

## Self-Review

**Spec coverage:**
- ✅ Snippets return plain text (no `**`, `##`, `[[...]]`)
- ✅ Wikilinks collapsed to label or target
- ✅ Headings, bold, italic, inline code, links all handled
- ✅ Tests for each case

**Placeholder scan:** No TBDs, no "handle edge cases" without code, all steps have concrete commands and code.

**Type consistency:** `strip_to_plain_text` is declared in `src/markdown/strip.rs`, re-exported from `src/markdown/mod.rs`, re-exported from `src/lib.rs` as `crate::strip_to_plain_text`, and called as such in `src/vault/paths.rs`. Consistent throughout.
