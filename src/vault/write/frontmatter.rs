//! In-place editing of a note's leading YAML frontmatter block.
//!
//! The block is edited as text, not regenerated. Only the lines a named key
//! owns are rewritten; every other byte between the `---` markers comes back
//! exactly as the author wrote it, including key order, flow versus block
//! sequences, indentation, quoting, comments, and blank lines (ADR-22).
//!
//! Replacing a named key rewrites every line that key owned, so a comment
//! written inside its value, or trailing its `key:` line, goes with the value
//! it annotated. That is the whole of what a caller can lose, and only for a
//! key the caller named.

use serde_json::{Map, Value};

use super::types::WriteError;

/// How a key's existing value is laid out, so a replacement can inherit it.
/// Only one layout is worth inheriting, because it is the only one whose
/// replacement would otherwise change how the note reads.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ValueShape {
    /// A block list under the key, with the indent its first `-` line uses.
    BlockList { indent: String },
    /// No block list to inherit: a one-line list, a scalar, a nested mapping,
    /// or a key that did not exist. A list written here goes on one line.
    NoBlockList,
}

/// One top-level key found in the block, and the lines it owns.
#[derive(Debug)]
struct KeyEntry {
    name: String,
    /// Index of the `key:` line.
    start: usize,
    /// One past the last line the key owns. Trailing blank and comment lines
    /// are deliberately excluded so they survive the key's deletion.
    end: usize,
    shape: ValueShape,
    /// The key's own text through its colon, exactly as written. Replacing a
    /// value reuses it rather than re-emitting the key, so a key the author
    /// wrote as `"a: b"` does not come back as `'a: b'`.
    prefix: String,
}

/// One line's worth of parse: the key it opens and whatever follows its colon.
struct KeyLine {
    name: String,
    /// The rest of the line after `key:`. Empty means the value is on the
    /// lines below, which is the only case where a block list can start.
    inline: String,
}

impl KeyLine {
    /// The line's text up to and including the colon.
    fn prefix<'a>(&self, line: &'a str) -> &'a str {
        &line[..line.len() - self.inline.len()]
    }
}

/// The result of applying `updates` to a frontmatter block's inner text.
pub(super) enum FrontmatterEdit {
    /// The block's new inner text, without a trailing newline, in the same
    /// convention as `cache::parse::frontmatter_span`.
    Block(String),
    /// Every key was deleted; the caller strips the block entirely.
    Empty,
}

/// Apply `updates` to `block`, the inner text of a note's frontmatter (empty
/// when the note has no block yet), touching only the keys `updates` names.
///
/// A `null` update deletes its key. Keys that did not exist are appended at the
/// end of the block, above any blank lines already trailing there, and a new
/// list is written on one line.
///
/// The whole call is refused, leaving the caller nothing to write, in three
/// cases: the block does not parse as a YAML mapping; a named key cannot be
/// located and replaced unambiguously, a key written twice in the same block
/// being the case that occurs in practice; or the edited text does not reparse
/// to the merge that was asked for. That last one is the backstop for a block
/// laid out in a way this scanner reads differently from a YAML parser, an
/// indented top-level mapping being the example, and it is the reason a caller
/// can be refused over a key it did not name. Short of that, a key the caller
/// did not name is never a reason to refuse.
pub(super) fn edit_frontmatter_block(
    block: &str,
    updates: &Map<String, Value>,
    relative_path: &str,
) -> Result<FrontmatterEdit, WriteError> {
    let original = parse_block(block, relative_path)?;
    let mut merged = original.clone();
    for (key, value) in updates {
        if value.is_null() {
            merged.remove(key);
        } else {
            merged.insert(key.clone(), value.clone());
        }
    }
    let lines: Vec<&str> = if block.is_empty() {
        Vec::new()
    } else {
        block.split('\n').collect()
    };
    let entries = top_level_keys(&lines);

    // Every named key has to be locatable exactly once before anything is
    // rewritten, so a refusal never half-applies. This runs before the
    // block-is-now-empty shortcut below, so that deleting an ambiguous key
    // refuses like changing one rather than taking the whole block with it.
    let mut located: Map<String, Value> = Map::new();
    let mut appended: Vec<(&String, &Value)> = Vec::new();
    for (key, value) in updates {
        let occurrences = entries.iter().filter(|entry| &entry.name == key).count();
        match occurrences {
            1 => {
                located.insert(key.clone(), value.clone());
            }
            0 => {
                // The scanner and the YAML parser must agree that the key is
                // absent. When they disagree the key is written in a form this
                // module cannot edit, and appending a second copy would change
                // a value the caller did not ask to change.
                if original.contains_key(key) {
                    return Err(unlocatable_key(
                        key,
                        "it is written in a form this tool cannot edit in place",
                        relative_path,
                    ));
                }
                if !value.is_null() {
                    appended.push((key, value));
                }
            }
            count => {
                return Err(unlocatable_key(
                    key,
                    &format!(
                        "it appears {count} times in this note's frontmatter, so which one to change is ambiguous"
                    ),
                    relative_path,
                ));
            }
        }
    }

    if merged.is_empty() {
        return Ok(FrontmatterEdit::Empty);
    }

    // A block read out of a CRLF file carries a `\r` on the end of every line.
    // Copied lines keep theirs for free; a line this module writes has to be
    // given one, or a one-key edit leaves the note with mixed line endings.
    let eol = if lines.iter().any(|line| line.ends_with('\r')) {
        "\r"
    } else {
        ""
    };
    let render = |key: &str, value: &Value, shape: &ValueShape, prefix: Option<&str>| {
        render_key(key, value, shape, prefix).map(|rendered| {
            rendered
                .into_iter()
                .map(|line| line + eol)
                .collect::<Vec<_>>()
        })
    };

    let mut output: Vec<String> = Vec::new();
    let mut line = 0;
    while line < lines.len() {
        let Some(entry) = entries.iter().find(|entry| entry.start == line) else {
            output.push(lines[line].to_string());
            line += 1;
            continue;
        };
        match located.get(&entry.name) {
            Some(Value::Null) => {}
            Some(value) => output.extend(render(
                &entry.name,
                value,
                &entry.shape,
                Some(&entry.prefix),
            )?),
            None => output.extend(lines[entry.start..entry.end].iter().map(|l| l.to_string())),
        }
        // Lines the key does not own (a following comment or blank line) are
        // copied by the loop's normal path on the next iterations.
        line = entry.end;
    }
    // A new key goes at the end of the block, but above any blank lines already
    // sitting there, so it does not open a gap the author did not write.
    let end_of_block = output
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(0, |last| last + 1);
    let mut new_keys: Vec<String> = Vec::new();
    for (key, value) in appended {
        new_keys.extend(render(key, value, &ValueShape::NoBlockList, None)?);
    }
    output.splice(end_of_block..end_of_block, new_keys);

    let edited = output.join("\n");
    // The canonical parser closes the block at the first `\n---`, so a value
    // serialized onto a line of its own containing only `---` would corrupt
    // every later read of the note. Refuse instead of writing.
    if edited.split('\n').any(|line| line.trim_end() == "---") {
        return Err(WriteError::InvalidInput(
            "updated frontmatter values may not serialize a line containing only '---'".to_string(),
        ));
    }
    // The edit is textual surgery on someone else's YAML, so prove it landed:
    // the block that comes back has to parse to exactly the merge that was
    // asked for, or nothing is written.
    if parse_block(&edited, relative_path).ok().as_ref() != Some(&merged) {
        return Err(WriteError::InvalidInput(format!(
            "note '{relative_path}' frontmatter could not be edited without changing keys that were not named; {FIX_IN_VAULT}"
        )));
    }
    Ok(FrontmatterEdit::Block(edited))
}

/// Every refusal here means the same thing to a caller: this note needs a
/// person, not a retry. They share one closing sentence so a caller reading
/// messages rather than the error code can still tell them apart as a group.
const FIX_IN_VAULT: &str = "fix it directly in the vault before updating it through the API";

fn unlocatable_key(key: &str, reason: &str, relative_path: &str) -> WriteError {
    WriteError::InvalidInput(format!(
        "cannot edit key '{key}' in note '{relative_path}': {reason}; {FIX_IN_VAULT}"
    ))
}

fn unserializable(error: serde_yaml_ng::Error) -> WriteError {
    WriteError::InvalidInput(format!(
        "updated frontmatter cannot be serialized as YAML: {error}"
    ))
}

fn parse_block(block: &str, relative_path: &str) -> Result<Map<String, Value>, WriteError> {
    if block.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_yaml_ng::from_str::<Value>(block) {
        Ok(Value::Object(properties)) => Ok(properties),
        Ok(_) | Err(_) => Err(WriteError::InvalidInput(format!(
            "note '{relative_path}' has invalid YAML frontmatter; {FIX_IN_VAULT}"
        ))),
    }
}

/// Every top-level `key:` line in the block, with the lines it owns.
///
/// A top-level key starts at column zero and its colon is followed by a space
/// or the end of the line, which is what makes it a mapping key rather than a
/// plain scalar such as `https://example.com`. Continuation lines - an indented
/// value, a block sequence's `-` items at column zero, a wrapped flow
/// collection - belong to the key above them.
fn top_level_keys(lines: &[&str]) -> Vec<KeyEntry> {
    let mut inlines: Vec<String> = Vec::new();
    let mut entries: Vec<KeyEntry> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(key) = top_level_key_line(line) else {
            continue;
        };
        if let Some(previous) = entries.last_mut() {
            previous.end = index;
        }
        let prefix = key.prefix(line).to_string();
        inlines.push(key.inline);
        entries.push(KeyEntry {
            name: key.name,
            start: index,
            end: lines.len(),
            shape: ValueShape::NoBlockList,
            prefix,
        });
    }
    for (entry, inline) in entries.iter_mut().zip(inlines) {
        // A blank or comment line sitting between two keys belongs to neither,
        // so it survives whatever happens to the key above it.
        while entry.end > entry.start + 1 {
            let candidate = lines[entry.end - 1].trim_start();
            if candidate.is_empty() || candidate.starts_with('#') {
                entry.end -= 1;
            } else {
                break;
            }
        }
        entry.shape = value_shape(&inline, &lines[entry.start + 1..entry.end]);
    }
    entries
}

fn top_level_key_line(line: &str) -> Option<KeyLine> {
    if line.starts_with(char::is_whitespace) || line.trim().is_empty() {
        return None;
    }
    if line.starts_with('#') || line.starts_with("- ") || line.trim_end() == "-" {
        return None;
    }
    let (name, rest) = if let Some(quote) = line.strip_prefix('"') {
        let (raw, rest) = split_quoted_key(quote, '"')?;
        (
            serde_yaml_ng::from_str::<String>(&format!("\"{raw}\"")).ok()?,
            rest,
        )
    } else if let Some(quote) = line.strip_prefix('\'') {
        let (raw, rest) = split_quoted_key(quote, '\'')?;
        (
            serde_yaml_ng::from_str::<String>(&format!("'{raw}'")).ok()?,
            rest,
        )
    } else {
        let (name, rest) = line.split_once(':')?;
        (name.to_string(), rest)
    };
    // `key:value` is a plain scalar in YAML, not a mapping entry.
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    if name.is_empty() {
        return None;
    }
    Some(KeyLine {
        name,
        inline: rest.to_string(),
    })
}

/// Split a quoted key from the rest of its line, returning the raw text inside
/// the quotes and everything after the `:` that must follow the closing quote.
fn split_quoted_key(after_quote: &str, quote: char) -> Option<(String, &str)> {
    let mut raw = String::new();
    let mut characters = after_quote.char_indices();
    while let Some((index, character)) = characters.next() {
        if character == '\\' && quote == '"' {
            raw.push(character);
            let (_, escaped) = characters.next()?;
            raw.push(escaped);
            continue;
        }
        if character == quote {
            let rest = &after_quote[index + character.len_utf8()..];
            return rest.strip_prefix(':').map(|rest| (raw, rest));
        }
        raw.push(character);
    }
    None
}

/// The shape of a key's value, given what followed its colon (`inline`) and
/// the lines below it (`below`). A block list can only start where the colon
/// was the end of the line, and a comment above the first `-` does not end it.
fn value_shape(inline: &str, below: &[&str]) -> ValueShape {
    if !inline.trim().is_empty() {
        return ValueShape::NoBlockList;
    }
    for line in below {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("- ") || trimmed.trim_end() == "-" {
            let indent = line[..line.len() - trimmed.len()].to_string();
            return ValueShape::BlockList { indent };
        }
        break;
    }
    ValueShape::NoBlockList
}

/// The lines a key and its new value occupy. The emitter does the quoting, so
/// a key written fresh is always correct; `original_prefix`, when the key
/// already exists, puts the author's own spelling of it back on the front.
fn render_key(
    key: &str,
    value: &Value,
    shape: &ValueShape,
    original_prefix: Option<&str>,
) -> Result<Vec<String>, WriteError> {
    let rendered = render_emitted(key, value, shape)?;
    let Some(prefix) = original_prefix else {
        return Ok(rendered);
    };
    // Every rendering opens with the emitter's own `key:`; swap it for the
    // author's. A key whose two spellings cannot be lined up keeps the
    // emitter's, which is still valid YAML naming the same key.
    let emitted_prefix = emit(key, &Value::Null)?;
    let Some(emitted_prefix) = emitted_prefix.trim_end().strip_suffix(" null") else {
        return Ok(rendered);
    };
    Ok(rendered
        .into_iter()
        .enumerate()
        .map(|(index, line)| match line.strip_prefix(emitted_prefix) {
            Some(rest) if index == 0 => format!("{prefix}{rest}"),
            _ => line,
        })
        .collect())
}

fn render_emitted(key: &str, value: &Value, shape: &ValueShape) -> Result<Vec<String>, WriteError> {
    if let Value::Array(items) = value {
        // An empty block sequence is not expressible in YAML, so an emptied
        // list falls through to the one-line form whatever shape it had.
        if let ValueShape::BlockList { indent } = shape
            && !items.is_empty()
        {
            let emitted = emit(key, value)?;
            let mut lines = emitted.lines();
            let head = lines.next().unwrap_or_default().to_string();
            let mut rendered = vec![head];
            rendered.extend(lines.map(|line| format!("{indent}{line}")));
            return Ok(rendered);
        }
        // House style for anything written fresh: one line.
        let head = emit(key, &Value::Array(Vec::new()))?;
        let head = head.trim_end_matches('\n');
        let opening = head.strip_suffix("[]").ok_or_else(|| {
            WriteError::InvalidInput(format!("frontmatter key '{key}' cannot be serialized"))
        })?;
        return Ok(vec![format!("{opening}[{}]", flow_items(items)?)]);
    }
    Ok(emit(key, value)?
        .trim_end_matches('\n')
        .split('\n')
        .map(str::to_string)
        .collect())
}

fn emit(key: &str, value: &Value) -> Result<String, WriteError> {
    let mut single = Map::new();
    single.insert(key.to_string(), value.clone());
    serde_yaml_ng::to_string(&single).map_err(unserializable)
}

/// The inside of a one-line sequence. An item is written plainly where the flow
/// context reads it back unchanged, and JSON-quoted where it would not - JSON
/// being a subset of YAML, that is always a legal escape hatch.
fn flow_items(items: &[Value]) -> Result<String, WriteError> {
    let mut rendered = Vec::with_capacity(items.len());
    for item in items {
        let plain = serde_yaml_ng::to_string(item)
            .map_err(unserializable)?
            .trim_end_matches('\n')
            .to_string();
        if is_flow_safe(&plain) {
            rendered.push(plain);
        } else {
            rendered.push(serde_json::to_string(item).map_err(|error| {
                WriteError::InvalidInput(format!("frontmatter value cannot be written: {error}"))
            })?);
        }
    }
    Ok(rendered.join(", "))
}

fn is_flow_safe(rendered: &str) -> bool {
    !rendered.is_empty()
        && rendered.trim() == rendered
        && !rendered.contains(|character: char| {
            character.is_control()
                || matches!(
                    character,
                    ',' | '['
                        | ']'
                        | '{'
                        | '}'
                        | '#'
                        | '&'
                        | '*'
                        | '!'
                        | '|'
                        | '>'
                        | '%'
                        | '@'
                        | '`'
                        | '"'
                        | '\''
                        | ':'
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(block: &str, updates: serde_json::Value) -> Result<String, WriteError> {
        let Value::Object(updates) = updates else {
            panic!("updates must be an object");
        };
        match edit_frontmatter_block(block, &updates, "Note")? {
            FrontmatterEdit::Block(block) => Ok(block),
            FrontmatterEdit::Empty => Ok(String::new()),
        }
    }

    #[test]
    fn an_unnamed_key_keeps_its_order_and_its_one_line_list() {
        let block = "tags: [type/project, status/active]\ncreated: 2026-03-18";
        let edited = edit(block, serde_json::json!({"status": "active"})).expect("edit");
        assert_eq!(
            edited,
            "tags: [type/project, status/active]\ncreated: 2026-03-18\nstatus: active"
        );
    }

    #[test]
    fn a_comment_and_a_blank_line_survive_a_neighbouring_delete() {
        let block = "one: 1\n\n# about two\ntwo: 2\nthree: 3";
        let edited = edit(block, serde_json::json!({"two": null})).expect("edit");
        assert_eq!(edited, "one: 1\n\n# about two\nthree: 3");
    }

    #[test]
    fn a_replacement_inherits_the_shape_the_author_used() {
        let block = "flow: [a, b]\nblock:\n  - a\n  - b";
        let edited = edit(
            block,
            serde_json::json!({"flow": ["x", "y"], "block": ["x", "y"]}),
        )
        .expect("edit");
        assert_eq!(edited, "flow: [x, y]\nblock:\n  - x\n  - y");
    }

    #[test]
    fn a_duplicate_key_is_refused_by_name_only_when_it_is_named() {
        let block = "keep: first\nkeep: second\nother: value";
        let error = edit(block, serde_json::json!({"keep": "third"}))
            .expect_err("a duplicate key cannot be edited unambiguously");
        let WriteError::InvalidInput(message) = error else {
            panic!("expected an invalid-input refusal");
        };
        assert!(message.contains("'keep'"), "{message}");
        assert!(message.contains("appears 2 times"), "{message}");

        let edited = edit(block, serde_json::json!({"other": "changed"})).expect("edit");
        assert_eq!(edited, "keep: first\nkeep: second\nother: changed");
    }

    #[test]
    fn a_url_value_is_not_mistaken_for_a_second_key() {
        let block = "home: value\nurl: https://example.com/a:b";
        let edited = edit(block, serde_json::json!({"home": "changed"})).expect("edit");
        assert_eq!(edited, "home: changed\nurl: https://example.com/a:b");
    }

    #[test]
    fn deleting_an_ambiguous_key_refuses_rather_than_taking_the_block_with_it() {
        let error = edit(
            "keep: first\nkeep: second",
            serde_json::json!({"keep": null}),
        )
        .expect_err("a duplicated key is as ambiguous to delete as it is to change");
        let WriteError::InvalidInput(message) = error else {
            panic!("expected an invalid-input refusal");
        };
        assert!(message.contains("'keep'"), "{message}");
    }

    #[test]
    fn an_emptied_list_falls_back_to_the_one_line_form() {
        // A block sequence with no items is not expressible in YAML, so the
        // shape cannot be inherited here whatever the author wrote.
        let edited = edit("tags:\n  - a\n  - b", serde_json::json!({"tags": []})).expect("edit");
        assert_eq!(edited, "tags: []");
    }

    #[test]
    fn a_crlf_block_keeps_its_line_endings_on_the_lines_this_module_writes() {
        let block = "title: Home\r\ntags: [a]\r";
        let edited =
            edit(block, serde_json::json!({"tags": ["b"], "status": "new"})).expect("edit");
        assert_eq!(edited, "title: Home\r\ntags: [b]\r\nstatus: new\r");
    }

    #[test]
    fn a_new_key_lands_above_a_trailing_blank_line_not_below_it() {
        let edited = edit("title: Home\n", serde_json::json!({"status": "new"})).expect("edit");
        assert_eq!(edited, "title: Home\nstatus: new\n");
    }

    #[test]
    fn a_new_key_lands_below_a_trailing_comment() {
        let edited = edit(
            "title: Home\n# a note to self",
            serde_json::json!({"status": "new"}),
        )
        .expect("edit");
        assert_eq!(edited, "title: Home\n# a note to self\nstatus: new");
    }

    #[test]
    fn a_block_whose_keys_are_all_indented_is_refused_rather_than_half_edited() {
        // Valid YAML that this scanner finds no top-level key in. Naming an
        // existing key is refused for being unlocatable; adding a new one is
        // caught by the reparse check, because appending at column zero would
        // change what the block means.
        let block = "  title: Home\n  tags: [a]";
        for updates in [
            serde_json::json!({"title": "Changed"}),
            serde_json::json!({"status": "new"}),
        ] {
            let error = edit(block, updates).expect_err("this block is not safely editable");
            assert!(matches!(error, WriteError::InvalidInput(_)), "{error:?}");
        }
    }

    #[test]
    fn a_quoted_key_holding_a_colon_still_has_its_block_list_seen() {
        let block = "\"a: b\":\n  - one";
        let edited = edit(block, serde_json::json!({"a: b": ["two"]})).expect("edit");
        assert_eq!(edited, "\"a: b\":\n  - two");
    }

    #[test]
    fn a_flow_item_that_needs_quoting_gets_it() {
        let edited = edit(
            "a: 1",
            serde_json::json!({"tags": ["plain", "key: value", "x,y"]}),
        )
        .expect("edit");
        assert_eq!(edited, "a: 1\ntags: [plain, \"key: value\", \"x,y\"]");
    }
}
