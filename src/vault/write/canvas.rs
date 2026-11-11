//! Keeping Obsidian canvases pointed at a file Hatchdoor renames or moves
//! (ADR-53).
//!
//! A canvas is a `.canvas` file in the JSON Canvas format: a board whose
//! cards are `nodes`. A card that shows a file is a node with
//! `"type": "file"` whose `file` value is that file's Vault-relative path.
//! Nothing else about a canvas is read here, and nothing else is written:
//! the planner swaps the characters between one value's quotes and copies
//! every other byte (ADR-22).

use std::ops::Range;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::cache::parse::content_hash;
use crate::vault::types::VaultIndex;
use crate::vault::{BoundedReadError, MAX_NOTE_BYTES, read_bounded};

use super::types::TextRewrite;

/// The extension that makes a file a canvas, compared without case.
const CANVAS_EXTENSION: &str = "canvas";

/// Whether `path` names a canvas file.
pub(super) fn is_canvas_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(CANVAS_EXTENSION))
}

/// A canvas the planner left alone although it may name a moved file, and
/// why. It never stops the operation: it is reported beside the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SkippedCanvas {
    pub(super) relative_path: String,
    pub(super) reason: String,
}

impl SkippedCanvas {
    /// The sentence a caller is shown for this canvas.
    pub(super) fn warning(&self) -> String {
        format!(
            "Canvas '{}' was not updated: {}",
            self.relative_path, self.reason
        )
    }
}

/// What a move means for the Vault's canvases: the rewrites that join the
/// operation's plan, and the canvases that could not be judged or edited.
#[derive(Debug, Default)]
pub(super) struct CanvasPlan {
    pub(super) rewrites: Vec<TextRewrite>,
    pub(super) skipped: Vec<SkippedCanvas>,
}

/// Plan the canvas rewrites for one operation's file moves.
///
/// `moves` are the operation's own, source and destination, the moving note
/// or attachment and every asset travelling with it. Each canvas the index
/// catalogued is read under the note size bound, and one whose `file` nodes
/// name a source gets a rewrite to the destination. A canvas that is itself
/// moving is rewritten where the move will have put it, because rewrites are
/// applied after the moves.
///
/// Nothing here refuses: a canvas that cannot be read as JSON, or is over
/// the bound, lands in [`CanvasPlan::skipped`] and the operation goes on
/// (ADR-53, decision 5).
pub(super) fn canvas_rewrite_plan(
    vault_root: &Path,
    index: &VaultIndex,
    moves: &[(&Path, &Path)],
) -> CanvasPlan {
    let renames = moves
        .iter()
        .filter_map(|(source, destination)| {
            Some((
                vault_relative(vault_root, source)?,
                vault_relative(vault_root, destination)?,
            ))
        })
        .collect::<Vec<_>>();
    let mut plan = CanvasPlan::default();
    if renames.is_empty() {
        return plan;
    }
    for relative_path in &index.asset_paths {
        let path = vault_root.join(relative_path);
        if !is_canvas_path(&path) {
            continue;
        }
        let mut skip = |reason: String| {
            let skipped = SkippedCanvas {
                relative_path: relative_path.clone(),
                reason,
            };
            tracing::warn!(canvas = %skipped.relative_path, reason = %skipped.reason, "Skipping a canvas during a move");
            plan.skipped.push(skipped);
        };
        let bytes = match read_bounded(&path, MAX_NOTE_BYTES) {
            Ok(bytes) => bytes,
            Err(BoundedReadError::TooLarge { limit }) => {
                skip(format!(
                    "it is larger than {} MB, so it was not read",
                    limit / (1024 * 1024)
                ));
                continue;
            }
            // A canvas that cannot be opened is skipped as an unreadable
            // note is: there is nothing to judge it by.
            Err(_) => continue,
        };
        let content = match String::from_utf8(bytes) {
            Ok(content) => content,
            Err(error) => {
                let lossy = String::from_utf8_lossy(error.as_bytes());
                if mentions_any(&lossy, &renames) {
                    skip("it is not valid UTF-8 text".to_string());
                }
                continue;
            }
        };
        match retarget_canvas(&content, &renames) {
            CanvasEdit::Unchanged => {}
            CanvasEdit::Rewritten(rewritten) => {
                let destination = moves
                    .iter()
                    .find(|(source, _)| *source == path)
                    .map(|(_, destination)| destination.to_path_buf());
                plan.rewrites.push(TextRewrite {
                    path: destination.unwrap_or(path),
                    original_hash: content_hash(&content),
                    content: rewritten,
                });
            }
            CanvasEdit::Unreadable(reason) => {
                if mentions_any(&content, &renames) {
                    skip(reason.to_string());
                }
            }
        }
    }
    plan
}

/// Split the paths a journal reports as rewritten into notes and canvases,
/// keeping each list's order.
pub(super) fn split_rewritten(rewritten: Vec<PathBuf>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    rewritten
        .into_iter()
        .partition(|path| !is_canvas_path(path))
}

fn vault_relative(vault_root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(vault_root).ok()?;
    Some(relative.to_str()?.replace('\\', "/"))
}

/// Whether `text` holds any old path, as written or as JSON escapes it. Only
/// used to decide whether a canvas that cannot be parsed is worth naming.
fn mentions_any(text: &str, renames: &[(String, String)]) -> bool {
    renames
        .iter()
        .any(|(old, _)| text.contains(old.as_str()) || text.contains(json_escaped(old).as_str()))
}

/// `value` as the inside of a JSON string, without the quotes.
fn json_escaped(value: &str) -> String {
    let quoted = Value::String(value.to_string()).to_string();
    quoted[1..quoted.len() - 1].to_string()
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CanvasEdit {
    /// No `file` node names a moved file.
    Unchanged,
    /// The canvas text with each matching path replaced and nothing else.
    Rewritten(String),
    /// The canvas cannot be edited safely, and why.
    Unreadable(&'static str),
}

/// Replace, in a canvas's text, the path of every `file` node that names an
/// old path in `renames` with its new path.
///
/// The edit is text surgery, so it proves its own work as ADR-22 asks: the
/// result is parsed again and must equal the original document with exactly
/// those values replaced.
pub(super) fn retarget_canvas(text: &str, renames: &[(String, String)]) -> CanvasEdit {
    let Ok(document) = serde_json::from_str::<Value>(text) else {
        return CanvasEdit::Unreadable("it is not valid JSON");
    };
    let new_path = |old: &str| {
        renames
            .iter()
            .find(|(from, _)| from == old)
            .map(|(_, to)| to.as_str())
    };

    let mut expected = document;
    let mut expected_changes = 0usize;
    if let Some(nodes) = expected.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in nodes {
            if node.get("type").and_then(Value::as_str) != Some("file") {
                continue;
            }
            let Some(file) = node.get_mut("file") else {
                continue;
            };
            if let Some(new) = file.as_str().and_then(new_path) {
                *file = Value::String(new.to_string());
                expected_changes += 1;
            }
        }
    }
    if expected_changes == 0 {
        return CanvasEdit::Unchanged;
    }

    const UNSAFE: CanvasEdit = CanvasEdit::Unreadable("its file cards could not be located safely");
    let Some(spans) = Scanner::new(text).file_value_spans() else {
        return UNSAFE;
    };
    let mut rewritten = String::with_capacity(text.len());
    let mut copied = 0usize;
    for span in spans {
        let Ok(Value::String(current)) = serde_json::from_str::<Value>(&text[span.clone()]) else {
            return UNSAFE;
        };
        let Some(new) = new_path(&current) else {
            continue;
        };
        // The span covers the quotes; they are copied, the text between them
        // is replaced.
        rewritten.push_str(&text[copied..span.start + 1]);
        rewritten.push_str(&json_escaped(new));
        copied = span.end - 1;
    }
    rewritten.push_str(&text[copied..]);
    match serde_json::from_str::<Value>(&rewritten) {
        Ok(reparsed) if reparsed == expected => CanvasEdit::Rewritten(rewritten),
        _ => UNSAFE,
    }
}

/// A walk over JSON text that remembers where things are, which a parsed
/// `Value` does not. It runs only over text `serde_json` has accepted, so a
/// surprise means this scanner and the parser disagree, and every method
/// answers `None` rather than guess.
struct Scanner<'a> {
    text: &'a str,
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            bytes: text.as_bytes(),
            position: 0,
        }
    }

    /// The byte range, quotes included, of each string `file` value in an
    /// object of the top-level `nodes` list whose `type` is `"file"`, in
    /// text order.
    fn file_value_spans(mut self) -> Option<Vec<Range<usize>>> {
        let mut spans = Vec::new();
        self.skip_whitespace();
        if self.peek()? != b'{' {
            return Some(spans);
        }
        self.members(|scanner, key| {
            if key == "nodes" && scanner.peek()? == b'[' {
                scanner.elements(|scanner| scanner.node(&mut spans))
            } else {
                scanner.skip_value()
            }
        })?;
        Some(spans)
    }

    /// One element of `nodes`: collect its `file` spans when it is a file
    /// node.
    fn node(&mut self, spans: &mut Vec<Range<usize>>) -> Option<()> {
        if self.peek()? != b'{' {
            return self.skip_value();
        }
        let mut is_file_node = false;
        let mut files = Vec::new();
        self.members(|scanner, key| {
            let start = scanner.position;
            let is_string = scanner.peek()? == b'"';
            scanner.skip_value()?;
            let span = start..scanner.position;
            if is_string && key == "type" {
                // A repeated key keeps its last value, as the parser does.
                is_file_node = scanner.decode(span)? == "file";
            } else if is_string && key == "file" {
                files.push(span);
            }
            Some(())
        })?;
        if is_file_node {
            spans.extend(files);
        }
        Some(())
    }

    /// Walk the object at the cursor, handing each member's decoded key to
    /// `visit` with the cursor on its value. `visit` must consume the value.
    fn members(&mut self, mut visit: impl FnMut(&mut Self, &str) -> Option<()>) -> Option<()> {
        self.expect(b'{')?;
        self.skip_whitespace();
        if self.peek()? == b'}' {
            self.position += 1;
            return Some(());
        }
        loop {
            self.skip_whitespace();
            let start = self.position;
            self.skip_string()?;
            let key = self.decode(start..self.position)?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            visit(self, &key)?;
            self.skip_whitespace();
            match self.next()? {
                b',' => continue,
                b'}' => return Some(()),
                _ => return None,
            }
        }
    }

    /// Walk the array at the cursor, calling `visit` with the cursor on each
    /// element. `visit` must consume the element.
    fn elements(&mut self, mut visit: impl FnMut(&mut Self) -> Option<()>) -> Option<()> {
        self.expect(b'[')?;
        self.skip_whitespace();
        if self.peek()? == b']' {
            self.position += 1;
            return Some(());
        }
        loop {
            self.skip_whitespace();
            visit(self)?;
            self.skip_whitespace();
            match self.next()? {
                b',' => continue,
                b']' => return Some(()),
                _ => return None,
            }
        }
    }

    fn skip_value(&mut self) -> Option<()> {
        match self.peek()? {
            b'"' => self.skip_string(),
            b'{' => self.members(|scanner, _| scanner.skip_value()),
            b'[' => self.elements(Self::skip_value),
            _ => {
                // A number or a literal: it ends at the next structural byte.
                let start = self.position;
                while self
                    .peek()
                    .is_some_and(|byte| !matches!(byte, b',' | b'}' | b']') && !is_whitespace(byte))
                {
                    self.position += 1;
                }
                (self.position > start).then_some(())
            }
        }
    }

    fn skip_string(&mut self) -> Option<()> {
        self.expect(b'"')?;
        loop {
            match self.next()? {
                b'"' => return Some(()),
                // Whatever follows a backslash is part of the escape, a
                // quote included.
                b'\\' => {
                    self.next()?;
                }
                _ => {}
            }
        }
    }

    /// The string a quoted span spells.
    fn decode(&self, span: Range<usize>) -> Option<String> {
        match serde_json::from_str::<Value>(self.text.get(span)?) {
            Ok(Value::String(value)) => Some(value),
            _ => None,
        }
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(is_whitespace) {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.position += 1;
        Some(byte)
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        (self.next()? == byte).then_some(())
    }
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renames(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(old, new)| (old.to_string(), new.to_string()))
            .collect()
    }

    #[test]
    fn only_the_matching_path_changes_and_every_other_byte_survives() {
        // Odd whitespace, an unknown field, a key order nobody would emit,
        // CRLF, a tab and no trailing newline.
        let canvas = "{\r\n\t\"edges\" : [ ],\n  \"nodes\":[\n    {\"file\"  :  \"Notes/Old.md\" ,\"x\":1e2,\"type\":\"file\", \"mystery\": {\"file\": \"Notes/Old.md\"}},\n    {\"type\":\"text\",\"text\":\"see Notes/Old.md\",\"file\":\"Notes/Old.md\"},\n    {\"type\":\"file\",\"file\":\"Notes/Other.md\",\"subpath\":\"#Notes/Old.md\"}\n  ] ,\"future\":true}";

        let edit = retarget_canvas(canvas, &renames(&[("Notes/Old.md", "Archive/New name.md")]));

        let expected = canvas.replacen(
            "\"file\"  :  \"Notes/Old.md\"",
            "\"file\"  :  \"Archive/New name.md\"",
            1,
        );
        assert_eq!(edit, CanvasEdit::Rewritten(expected));
    }

    #[test]
    fn a_canvas_naming_no_moved_file_is_unchanged() {
        let canvas = r#"{"nodes":[{"type":"file","file":"Notes/Other.md"}]}"#;
        assert_eq!(
            retarget_canvas(canvas, &renames(&[("Notes/Old.md", "Notes/New.md")])),
            CanvasEdit::Unchanged
        );
        for text in ["{}", "[]", "null", r#"{"nodes":{"file":"Notes/Old.md"}}"#] {
            assert_eq!(
                retarget_canvas(text, &renames(&[("Notes/Old.md", "Notes/New.md")])),
                CanvasEdit::Unchanged,
                "{text}"
            );
        }
    }

    #[test]
    fn an_escaped_path_is_matched_and_the_new_path_is_escaped() {
        let canvas = r#"{"nodes":[{"type":"file","file":"Notes\/Café \"one\".md"}]}"#;

        let edit = retarget_canvas(
            canvas,
            &renames(&[("Notes/Café \"one\".md", "Notes/Back\\slash \"two\".md")]),
        );

        assert_eq!(
            edit,
            CanvasEdit::Rewritten(
                r#"{"nodes":[{"type":"file","file":"Notes/Back\\slash \"two\".md"}]}"#.to_string()
            )
        );
    }

    #[test]
    fn several_moves_are_applied_once_each_without_chaining() {
        let canvas = r#"{"nodes":[{"type":"file","file":"a.md"},{"type":"file","file":"b.md"}]}"#;

        let edit = retarget_canvas(canvas, &renames(&[("a.md", "b.md"), ("b.md", "c.md")]));

        assert_eq!(
            edit,
            CanvasEdit::Rewritten(
                r#"{"nodes":[{"type":"file","file":"b.md"},{"type":"file","file":"c.md"}]}"#
                    .to_string()
            )
        );
    }

    #[test]
    fn invalid_json_is_unreadable() {
        assert_eq!(
            retarget_canvas(
                r#"{"nodes":[{"type":"file","file":"a.md"},]}"#,
                &renames(&[("a.md", "b.md")])
            ),
            CanvasEdit::Unreadable("it is not valid JSON")
        );
    }

    #[test]
    fn a_repeated_key_is_edited_only_where_the_parser_agrees() {
        // The parser keeps the last `file`, so both spellings are replaced
        // and the reparse still matches.
        let canvas = r#"{"nodes":[{"type":"file","file":"a.md","file":"a.md"}]}"#;
        assert_eq!(
            retarget_canvas(canvas, &renames(&[("a.md", "b.md")])),
            CanvasEdit::Rewritten(
                r#"{"nodes":[{"type":"file","file":"b.md","file":"b.md"}]}"#.to_string()
            )
        );
        // The parser sees a text node, the first `type` notwithstanding.
        let canvas = r#"{"nodes":[{"type":"file","file":"a.md","type":"text"}]}"#;
        assert_eq!(
            retarget_canvas(canvas, &renames(&[("a.md", "b.md")])),
            CanvasEdit::Unchanged
        );
    }
}
