//! Finding every Note whose text contains one literal string (ADR-46).
//!
//! A text match *selects*, like a query, and never ranks: a Note either
//! contains the string or it does not, each matching Note reports how many
//! times, and the Notes come back in the stable order a query uses. Unlike
//! every other collection read it answers from the Markdown files on disk
//! rather than from the published snapshot, because its job is to verify an
//! edit and an answer that lags by one Index turn defeats that. Only the list
//! of Notes to read comes from the snapshot.

use std::io;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::cache::parse::frontmatter_span;
use crate::cache::vault_snapshots::{VaultSnapshotNote, VaultSnapshotRead};
use crate::search::LayerSelection;
use crate::vault_registry::VaultId;

use super::VaultReadError;
use super::query::{CompiledCondition, MAX_PATH_PREFIX_BYTES};

/// The longest string one text match may look for. A caller verifying a
/// rename or an ID never comes near it; a request past it is a client bug.
const MAX_TEXT_BYTES: usize = 4_096;

/// How many snippets a matching Note shows unless the caller asks for fewer.
/// Also the most it can ask for.
const MAX_SNIPPETS_PER_NOTE: usize = 3;

/// About how many characters of a matched line one snippet carries.
const SNIPPET_CHARS: usize = 200;

/// The most unread Notes one reply lists. `total_unread` still counts them all.
pub(super) const MAX_UNREAD_LISTED: usize = 50;

/// How many Notes a text match returns unless the caller asks otherwise, and
/// the most it can ask for. There is no paging past the ceiling (ADR-46).
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;

fn clamp_text_match_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// One request for every Note containing `text`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextMatchRequest {
    /// The literal string. Every character means itself.
    pub text: String,
    /// `false` ignores case. Accents count either way, and the composed and
    /// decomposed forms of one letter are equal either way.
    pub case_sensitive: bool,
    /// Layer selector tokens, in `search_notes`' grammar. Empty covers every
    /// layer, which is this operation's default and not
    /// `LayerSelection::default()`'s.
    pub layers: Vec<String>,
    /// Only Notes at or under this folder, matched the way a query's
    /// `path_prefix` condition is.
    pub path_prefix: Option<String>,
    /// Maximum Notes to return. `None` takes the default; anything outside
    /// the bounds clamps into them.
    pub limit: Option<usize>,
    /// Snippets shown per Note, `0` to `3`. `None` takes three.
    pub snippets_per_note: Option<usize>,
}

/// Which part of a Note a match sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextMatchPlace {
    /// The Markdown below the frontmatter block.
    Body,
    /// The leading frontmatter block, delimiters included.
    Frontmatter,
    /// The Vault-relative file path, extension included.
    Path,
}

/// One matching Note's occurrences, split by where they sit. The three add up
/// to the Note's `occurrences`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct TextMatchPlaces {
    pub body: usize,
    pub frontmatter: usize,
    pub path: usize,
}

/// One matched line, or the matched path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextMatchSnippet {
    pub place: TextMatchPlace,
    /// The 1-based line of the file the match starts on. Absent for a path
    /// match, which has no line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// The matched line as written, trimmed, and cut to about 200 characters
    /// around the match when it is longer.
    pub text: String,
}

/// One Note that contains the string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextMatchNote {
    pub vault_id: VaultId,
    pub title: String,
    pub slug: String,
    pub relative_path: String,
    /// The Note's layer, `null` on the default surface.
    pub layer: Option<String>,
    /// How many times the string occurs in this Note, over all three places.
    pub occurrences: usize,
    pub places: TextMatchPlaces,
    pub snippets: Vec<TextMatchSnippet>,
}

/// Why a Note's file could not be read when the match ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextMatchUnreadReason {
    /// The published Note list names a file that is no longer there.
    Missing,
    /// The file is there and could not be read.
    Unreadable,
}

/// One Note the match could not read, so it says nothing about that Note's
/// text. Its path was still checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextMatchUnread {
    pub vault_id: VaultId,
    pub slug: String,
    pub relative_path: String,
    pub reason: TextMatchUnreadReason,
}

/// A text match's answer, inside the shared collection envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextMatchResponse {
    pub notes: Vec<TextMatchNote>,
    /// Every matching Note in the scope, counted before `limit` applied.
    pub total_notes: usize,
    /// Every occurrence in the scope, counted before `limit` applied.
    pub total_occurrences: usize,
    /// Whether more Notes matched than `limit` let through.
    pub truncated: bool,
    /// Notes whose file could not be read, at most 50 of them. A Note listed
    /// here was not checked, so its absence from `notes` proves nothing.
    pub unread: Vec<TextMatchUnread>,
    /// How many Notes could not be read, whatever `unread` had room for.
    pub total_unread: usize,
}

/// A validated text match, ready to run against any number of Vaults.
#[derive(Debug)]
pub(super) struct CompiledTextMatch {
    needle: String,
    case_sensitive: bool,
    pub(super) layers: LayerSelection,
    folder: Option<CompiledCondition>,
    pub(super) limit: usize,
    snippets_per_note: usize,
}

/// What one Vault contributes: its first `limit` matching Notes in path
/// order, and the true counts over all of them.
#[derive(Debug, Default)]
pub(super) struct VaultTextMatches {
    pub(super) notes: Vec<TextMatchNote>,
    pub(super) total_notes: usize,
    pub(super) total_occurrences: usize,
    pub(super) unread: Vec<TextMatchUnread>,
    pub(super) total_unread: usize,
    /// The layer names this Vault declares, for validating a named selection
    /// across every participant.
    pub(super) declared_layers: Vec<String>,
}

impl CompiledTextMatch {
    /// Validate one request before any Vault is touched, so a malformed one
    /// is refused identically whatever its scope. `layers` arrives already
    /// resolved against the caller's browse surface.
    pub(super) fn compile(
        request: &TextMatchRequest,
        layers: LayerSelection,
    ) -> Result<Self, VaultReadError> {
        if request.text.is_empty() {
            return Err(invalid_text_match("text cannot be empty"));
        }
        if request.text.len() > MAX_TEXT_BYTES {
            return Err(invalid_text_match(format!(
                "text cannot exceed {MAX_TEXT_BYTES} bytes"
            )));
        }
        let folder = match &request.path_prefix {
            None => None,
            Some(prefix) if prefix.len() > MAX_PATH_PREFIX_BYTES => {
                return Err(invalid_text_match(format!(
                    "path_prefix cannot exceed {MAX_PATH_PREFIX_BYTES} bytes"
                )));
            }
            Some(prefix) => Some(
                CompiledCondition::folder(prefix)
                    .ok_or_else(|| invalid_text_match("path_prefix cannot be empty"))?,
            ),
        };
        Ok(Self {
            needle: fold(&request.text, request.case_sensitive),
            case_sensitive: request.case_sensitive,
            layers,
            folder,
            limit: clamp_text_match_limit(request.limit),
            snippets_per_note: request
                .snippets_per_note
                .unwrap_or(MAX_SNIPPETS_PER_NOTE)
                .min(MAX_SNIPPETS_PER_NOTE),
        })
    }

    /// Read every selected Note of one Vault from `root` and match it.
    pub(super) fn matches_for(
        &self,
        vault_id: VaultId,
        root: &Path,
        snapshot: &VaultSnapshotRead,
    ) -> VaultTextMatches {
        let mut selected: Vec<&VaultSnapshotNote> = snapshot
            .notes
            .iter()
            .filter(|note| self.covers_layer(note.layer.as_deref()))
            .filter(|note| {
                self.folder
                    .as_ref()
                    .is_none_or(|folder| folder.matches(note))
            })
            .collect();
        // The order `query::sort_rows` gives one Vault's rows, so keeping the
        // first `limit` here keeps the ones the merged answer can show.
        selected.sort_by(|left, right| {
            left.relative_path
                .cmp(&right.relative_path)
                .then_with(|| left.slug.cmp(&right.slug))
        });

        let mut found = VaultTextMatches {
            declared_layers: snapshot
                .layer_catalog
                .iter()
                .map(|layer| layer.name.clone())
                .collect(),
            ..VaultTextMatches::default()
        };
        // Resolved once, so each Note's own resolved path can be held against
        // it. A root that cannot be resolved leaves every Note unread.
        let root = std::fs::canonicalize(root).ok();
        for note in selected {
            let file_path = format!("{}.md", note.relative_path);
            let shown = found.notes.len() < self.limit;
            let snippets_wanted = if shown { self.snippets_per_note } else { 0 };
            let mut places = TextMatchPlaces::default();
            let mut snippets = Vec::new();

            match read_note_file(root.as_deref(), &file_path) {
                Ok(content) => {
                    (places.frontmatter, places.body) = self.scan(
                        &content,
                        body_start(&content),
                        snippets_wanted,
                        &mut snippets,
                    );
                }
                Err(reason) => {
                    found.total_unread += 1;
                    if found.unread.len() < MAX_UNREAD_LISTED {
                        found.unread.push(TextMatchUnread {
                            vault_id,
                            slug: note.slug.clone(),
                            relative_path: note.relative_path.clone(),
                            reason,
                        });
                    }
                }
            }

            places.path = self.count(&file_path);
            if places.path > 0 && snippets.len() < snippets_wanted {
                snippets.push(TextMatchSnippet {
                    place: TextMatchPlace::Path,
                    line: None,
                    text: file_path,
                });
            }

            let occurrences = places.body + places.frontmatter + places.path;
            if occurrences == 0 {
                continue;
            }
            found.total_notes += 1;
            found.total_occurrences += occurrences;
            if shown {
                found.notes.push(TextMatchNote {
                    vault_id,
                    title: note.title.clone(),
                    slug: note.slug.clone(),
                    relative_path: note.relative_path.clone(),
                    layer: note.layer.clone(),
                    occurrences,
                    places,
                    snippets,
                });
            }
        }
        found
    }

    fn covers_layer(&self, layer: Option<&str>) -> bool {
        match (&self.layers, layer) {
            (LayerSelection::All, _) => true,
            (
                LayerSelection::Set {
                    include_default, ..
                },
                None,
            ) => *include_default,
            (LayerSelection::Set { layers, .. }, Some(layer)) => layers.contains(layer),
        }
    }

    fn count(&self, text: &str) -> usize {
        fold(text, self.case_sensitive)
            .matches(self.needle.as_str())
            .count()
    }

    /// Count the occurrences in one Note's file, as (frontmatter, body), and
    /// add a snippet for each distinct matched line until the Note has
    /// `snippets_wanted` of them. `body_start` is where the body begins; an
    /// occurrence belongs to the place it starts in.
    ///
    /// Folding never adds or removes a line break, so a match's line in the
    /// folded text is its line in the file, and the snippet is cut from that
    /// line as written.
    fn scan(
        &self,
        content: &str,
        body_start: usize,
        snippets_wanted: usize,
        snippets: &mut Vec<TextMatchSnippet>,
    ) -> (usize, usize) {
        // Folded apart so the boundary is known in the folded text too. The
        // frontmatter block ends on a line break, so nothing composes across.
        let mut folded = fold(&content[..body_start], self.case_sensitive);
        let folded_body_start = folded.len();
        folded.push_str(&fold(&content[body_start..], self.case_sensitive));

        let (mut in_frontmatter, mut in_body) = (0, 0);
        // The line the scan has reached: its index, where it starts in the
        // folded text, and the same line as written.
        let mut line = 0;
        let mut line_start = 0;
        let mut written_lines = content.split('\n');
        let mut written_line = written_lines.next().unwrap_or("");
        let mut scanned = 0;
        let mut last_snippet_line = None;
        for (offset, _) in folded.match_indices(self.needle.as_str()) {
            let place = if offset < folded_body_start {
                in_frontmatter += 1;
                TextMatchPlace::Frontmatter
            } else {
                in_body += 1;
                TextMatchPlace::Body
            };
            if snippets.len() >= snippets_wanted {
                continue;
            }
            for (line_break, _) in folded[scanned..offset].match_indices('\n') {
                line += 1;
                line_start = scanned + line_break + 1;
                written_line = written_lines.next().unwrap_or("");
            }
            scanned = offset;
            if last_snippet_line == Some(line) {
                continue;
            }
            last_snippet_line = Some(line);
            snippets.push(TextMatchSnippet {
                place,
                line: Some(line + 1),
                text: self.snippet(written_line, &folded[line_start..offset]),
            });
        }
        (in_frontmatter, in_body)
    }

    /// One matched line trimmed, and cut to about [`SNIPPET_CHARS`]
    /// characters around the match when it is longer. `folded_before` is the
    /// folded text between the start of the line and the match.
    fn snippet(&self, line: &str, folded_before: &str) -> String {
        let line = line.trim_end_matches('\r');
        let characters: Vec<(usize, char)> = line.char_indices().collect();
        if characters.len() <= SNIPPET_CHARS {
            return line.trim().to_string();
        }
        // Folding can change how many characters a stretch of text has, so
        // the match's column as written is the shortest prefix that folds to
        // at least what came before the match.
        let before = folded_before.chars().count();
        let column = characters.partition_point(|(byte, _)| {
            fold(&line[..*byte], self.case_sensitive).chars().count() < before
        });
        // A quarter of the window before the match, the rest from it onward.
        let start = column
            .saturating_sub(SNIPPET_CHARS / 4)
            .min(characters.len() - SNIPPET_CHARS);
        let cut: String = characters[start..start + SNIPPET_CHARS]
            .iter()
            .map(|(_, character)| character)
            .collect();
        let mut text = String::new();
        if start > 0 {
            text.push('…');
        }
        text.push_str(cut.trim());
        if start + SNIPPET_CHARS < characters.len() {
            text.push('…');
        }
        text
    }
}

/// The form two strings are compared in: lowercased unless the match is
/// strict, then composed, so `é` typed as one character and as `e` plus a
/// combining accent are the same text. Each character is lowercased on its
/// own, because `str::to_lowercase` spells a Greek sigma by its position in
/// the word and a string would then stop matching inside a longer word.
fn fold(text: &str, case_sensitive: bool) -> String {
    if text.is_ascii() {
        return if case_sensitive {
            text.to_string()
        } else {
            text.to_ascii_lowercase()
        };
    }
    if case_sensitive {
        text.nfc().collect()
    } else {
        text.chars().flat_map(char::to_lowercase).nfc().collect()
    }
}

/// Where the body begins: just past the line closing a leading frontmatter
/// block, or `0` when the Note has none. The same block the canonical parser
/// and the write layer recognise.
fn body_start(content: &str) -> usize {
    let Some((_, inner_end)) = frontmatter_span(content) else {
        return 0;
    };
    // `inner_end` sits on the `\n` before the closing `---`.
    let closing = inner_end + 1;
    match content[closing..].find('\n') {
        Some(newline) => closing + newline + 1,
        None => content.len(),
    }
}

/// One Note's file as text, read from under the Vault's resolved `root`.
/// Bytes that are not UTF-8 read as the replacement character rather than
/// failing the Note.
///
/// The file is read only when its resolved path is the one the index named.
/// The index follows no symbolic link, so a link anywhere on the way arrived
/// after the index was built and may lead outside the Vault.
fn read_note_file(root: Option<&Path>, file_path: &str) -> Result<String, TextMatchUnreadReason> {
    let reason = |error: io::Error| match error.kind() {
        io::ErrorKind::NotFound => TextMatchUnreadReason::Missing,
        _ => TextMatchUnreadReason::Unreadable,
    };
    let path = root
        .ok_or(TextMatchUnreadReason::Unreadable)?
        .join(file_path);
    if std::fs::canonicalize(&path).map_err(reason)? != path {
        return Err(TextMatchUnreadReason::Unreadable);
    }
    let bytes = std::fs::read(&path).map_err(reason)?;
    Ok(match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    })
}

/// The stable order a text match answers in, the one `query::sort_rows` uses:
/// path, then Vault, then slug.
pub(super) fn sort_notes(notes: &mut [TextMatchNote]) {
    notes.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| left.vault_id.cmp(&right.vault_id))
            .then_with(|| left.slug.cmp(&right.slug))
    });
}

fn invalid_text_match(message: impl Into<String>) -> VaultReadError {
    VaultReadError {
        code: "invalid_text_match".to_string(),
        message: message.into(),
        vault_id: None,
        retryable: false,
    }
}
