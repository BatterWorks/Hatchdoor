//! The locating-sized search hit (#501): the fields that say which note a hit
//! is, plus a short snippet in place of the chunk.
//!
//! A compact hit is a projection of a full one. It is built after the search
//! has ranked and capped its results, so it cannot change which hits come
//! back, their scores or their order.

use schemars::JsonSchema;
use serde::Serialize;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

use crate::cache::parse::fts_query_terms;
use crate::vault_registry::VaultId;

use super::SearchResponseMode;
use super::vault_scoped::{VaultSearchResponse, VaultSearchResult};

/// The most characters a snippet holds, its ellipses included. Fixed: no
/// argument tunes it.
const SNIPPET_CHARS: usize = 200;

const ELLIPSIS: char = '…';

/// One search hit without its chunk, links or metadata. `get_note` returns
/// the content of the note it names.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CompactSearchResult {
    pub vault_id: VaultId,
    pub note_slug: String,
    pub note_title: String,
    pub note_path: String,
    pub heading_path: Option<String>,
    pub score: f32,
    pub layer: Option<String>,
    /// At most 200 characters. For a `#tag` query it is the line
    /// `Matched tag: #<tag>`. Otherwise it is copied verbatim from the
    /// matched chunk: centred on the first matched query word in a keyword
    /// hit, the start of the chunk in any other. `…` marks each side where
    /// text was dropped.
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CompactSearchResponse {
    pub mode: SearchResponseMode,
    pub results: Vec<CompactSearchResult>,
}

impl CompactSearchResponse {
    /// The compact form of `response`, which answered `query`. The query is
    /// read only to place a keyword hit's snippet on a word it matched.
    pub fn from_full(response: VaultSearchResponse, query: &str) -> Self {
        let phrases = match response.mode {
            SearchResponseMode::Keyword => query_phrases(query),
            SearchResponseMode::Semantic | SearchResponseMode::Tag => Vec::new(),
        };
        Self {
            mode: response.mode,
            results: response
                .results
                .into_iter()
                .map(|result| compact_result(result, &phrases))
                .collect(),
        }
    }
}

fn compact_result(result: VaultSearchResult, phrases: &[Vec<String>]) -> CompactSearchResult {
    CompactSearchResult {
        snippet: snippet(&result.content, phrases),
        vault_id: result.vault_id,
        note_slug: result.note_slug,
        note_title: result.note_title,
        note_path: result.note_path,
        heading_path: result.heading_path,
        score: result.score,
        layer: result.layer,
    }
}

/// The keyword query as the index sees it: one phrase per query word, each a
/// run of folded tokens. The keyword path quotes every word, so `well-known`
/// matches only where `well` is followed by `known`.
fn query_phrases(query: &str) -> Vec<Vec<String>> {
    fts_query_terms(query)
        .iter()
        .map(|term| {
            let chars = term.char_indices().collect::<Vec<_>>();
            tokens(&chars)
                .into_iter()
                .map(|token| token.folded)
                .collect::<Vec<_>>()
        })
        .filter(|phrase| !phrase.is_empty())
        .collect()
}

struct Token {
    /// Character offsets, end exclusive.
    start: usize,
    end: usize,
    folded: String,
}

/// Splits text the way the keyword index's tokenizer does (`unicode61
/// remove_diacritics 2`): a token is a run of letters and digits, compared
/// without case or accents. Only a Latin-style accent continues a token.
/// Every other mark ends one, a Devanagari or Thai vowel sign included, so
/// a word in those scripts is several tokens here as it is in the index.
fn tokens(chars: &[(usize, char)]) -> Vec<Token> {
    let is_letter_or_digit = |ch: char| ch.is_alphanumeric() && !is_combining_mark(ch);
    let is_accent = |ch: char| ('\u{300}'..='\u{36F}').contains(&ch);
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if !is_letter_or_digit(chars[index].1) {
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len()
            && (is_letter_or_digit(chars[index].1) || is_accent(chars[index].1))
        {
            index += 1;
        }
        let folded = chars[start..index]
            .iter()
            .flat_map(|&(_, ch)| std::iter::once(ch).nfd())
            .filter(|ch| !is_combining_mark(*ch))
            .flat_map(char::to_lowercase)
            .collect();
        tokens.push(Token {
            start,
            end: index,
            folded,
        });
    }
    tokens
}

/// Character offsets of the earliest place any phrase occurs in a form a
/// snippet can hold: an occurrence too long for the window with an ellipsis
/// on each side is passed over.
fn first_match(chars: &[(usize, char)], phrases: &[Vec<String>]) -> Option<(usize, usize)> {
    if phrases.is_empty() {
        return None;
    }
    let tokens = tokens(chars);
    (0..tokens.len()).find_map(|at| {
        phrases.iter().find_map(|phrase| {
            let run = tokens.get(at..at + phrase.len())?;
            let (start, end) = (run[0].start, run[run.len() - 1].end);
            (end - start <= SNIPPET_CHARS - 2
                && run
                    .iter()
                    .zip(phrase)
                    .all(|(token, word)| token.folded == *word))
            .then_some((start, end))
        })
    })
}

/// At most [`SNIPPET_CHARS`] characters of `content`, copied verbatim.
///
/// With a phrase that occurs in the chunk, the window is centred on its first
/// occurrence; otherwise it is the start of the chunk. A cut lands on
/// whitespace when there is some in the outer three quarters of the text
/// kept on that side, so a word survives whole. Text with none there (a script written
/// without spaces, a very long URL) is cut between characters instead, never
/// inside a character or between an emoji and its joiners.
fn snippet(content: &str, phrases: &[Vec<String>]) -> String {
    let text = content.trim();
    let chars = text.char_indices().collect::<Vec<_>>();
    let total = chars.len();
    if total <= SNIPPET_CHARS {
        return text.to_owned();
    }
    let slice = |start: usize, end: usize| {
        let byte = |at: usize| chars.get(at).map_or(text.len(), |&(byte, _)| byte);
        &text[byte(start)..byte(end)]
    };

    // One character is kept for the ellipsis.
    let head_end = end_cut(&chars, SNIPPET_CHARS - 1, 0);
    // A match the start of the chunk already shows needs no centring.
    let centred = first_match(&chars, phrases).filter(|&(_, end)| end > head_end);
    let Some((match_start, match_end)) = centred else {
        return format!("{}{ELLIPSIS}", slice(0, head_end));
    };

    let middle = match_start + (match_end - match_start) / 2;
    let mut window_start = middle.saturating_sub(SNIPPET_CHARS / 2);
    let mut window_end = window_start + SNIPPET_CHARS;
    if window_end >= total {
        window_end = total;
        window_start = total - SNIPPET_CHARS;
    }
    // The window can still begin at the chunk's first character, and then
    // nothing was dropped before it.
    let (lead, start) = if window_start == 0 {
        (String::new(), 0)
    } else {
        (
            ELLIPSIS.to_string(),
            start_cut(&chars, window_start + 1, match_start),
        )
    };
    if window_end == total {
        return format!("{lead}{}", slice(start, total));
    }
    let end = end_cut(&chars, window_end - 1, match_end - 1);
    format!("{lead}{}{ELLIPSIS}", slice(start, end))
}

/// Where a snippet may end, given that it must end in `(floor, limit]`: the
/// last word end in the three quarters of that range nearer `limit`, or
/// failing that the last place at or before `limit` that splits no
/// character. A word end further in is passed over, because taking it would
/// throw away nearly all the text on this side.
fn end_cut(chars: &[(usize, char)], limit: usize, floor: usize) -> usize {
    let nearest_kept = floor + (limit - floor) / 4;
    let word_end = (nearest_kept + 1..=limit)
        .rev()
        .find(|&at| chars[at].1.is_whitespace());
    if let Some(mut at) = word_end {
        while at > floor + 1 && chars[at - 1].1.is_whitespace() {
            at -= 1;
        }
        return at;
    }
    let mut at = limit;
    while at > floor + 1 && joined(chars, at) {
        at -= 1;
    }
    at
}

/// Where a snippet may start, given that it must start in `[from, ceiling]`:
/// the first word start in the three quarters of that range nearer `from`,
/// or failing that the first place at or after `from` that splits no
/// character and is not whitespace.
fn start_cut(chars: &[(usize, char)], from: usize, ceiling: usize) -> usize {
    let furthest_kept = from + 3 * (ceiling - from) / 4;
    let word_start = (from..=furthest_kept)
        .find(|&at| !chars[at].1.is_whitespace() && chars[at - 1].1.is_whitespace());
    if let Some(at) = word_start {
        return at;
    }
    let mut at = from;
    while at < ceiling && (joined(chars, at) || chars[at].1.is_whitespace()) {
        at += 1;
    }
    at
}

/// Whether the character at `at` renders as one with the character before
/// it, so a cut between them would show as a broken character.
fn joined(chars: &[(usize, char)], at: usize) -> bool {
    const ZERO_WIDTH_JOINER: char = '\u{200D}';
    let is_regional_indicator = |ch: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&ch);
    let (previous, current) = (chars[at - 1].1, chars[at].1);
    // A flag is a pair of regional indicators, so in a run of them only an
    // odd number before the cut leaves one half of a flag on each side.
    let splits_a_flag = is_regional_indicator(current)
        && chars[..at]
            .iter()
            .rev()
            .take_while(|&&(_, ch)| is_regional_indicator(ch))
            .count()
            % 2
            == 1;
    // Combining marks include the variation selectors.
    is_combining_mark(current)
        || current == ZERO_WIDTH_JOINER
        || previous == ZERO_WIDTH_JOINER
        // Emoji skin tones.
        || ('\u{1F3FB}'..='\u{1F3FF}').contains(&current)
        // The tag characters that spell a subdivision flag.
        || ('\u{E0020}'..='\u{E007F}').contains(&current)
        // The vowel and final of a Hangul syllable stored decomposed.
        || ('\u{1160}'..='\u{11FF}').contains(&current)
        || splits_a_flag
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(word: &str, count: usize) -> String {
        vec![word; count].join(" ")
    }

    #[test]
    fn a_chunk_shorter_than_the_snippet_length_comes_back_whole() {
        assert_eq!(
            snippet("# Home\n\nA short note.", &[]),
            "# Home\n\nA short note."
        );
    }

    #[test]
    fn without_a_match_the_snippet_is_the_start_of_the_chunk_cut_at_a_word() {
        let text = words("word", 100);
        assert_eq!(snippet(&text, &[]), format!("{}…", words("word", 40)));
        assert_eq!(snippet(&text, &[]).chars().count(), 200);
    }

    #[test]
    fn a_keyword_far_into_a_long_chunk_sits_in_the_middle_of_its_snippet() {
        let text = format!("{} needle {}", words("alpha", 100), words("omega", 100));
        let found = snippet(&text, &query_phrases("needle"));
        assert_eq!(
            found,
            format!("…{} needle {}…", words("alpha", 16), words("omega", 16))
        );
        assert_eq!(found.chars().count(), 200);
    }

    #[test]
    fn a_keyword_the_start_of_the_chunk_already_shows_is_not_centred() {
        let text = format!("needle {}", words("omega", 100));
        assert_eq!(
            snippet(&text, &query_phrases("needle")),
            format!("needle {}…", words("omega", 32))
        );
    }

    /// The start-of-chunk snippet ends at a word break, which can fall just
    /// before a match that sits inside the first 200 characters.
    #[test]
    fn a_keyword_the_start_of_chunk_cut_would_drop_is_centred_instead() {
        // `needle,` occupies characters 192 to 198 and its word runs past 199.
        let text = format!(
            "{} needle,and-then-some {}",
            words("abc", 48),
            words("omega", 100)
        );
        assert_eq!(snippet(&text, &[]), format!("{}…", words("abc", 48)));
        assert_eq!(
            snippet(&text, &query_phrases("needle")),
            format!(
                "…{} needle,and-then-some {}…",
                words("abc", 24),
                words("omega", 13)
            )
        );
    }

    /// Found live (#501): Thai has no spaces, so the only word breaks near a
    /// Latin keyword were the two around it, and taking them left a snippet
    /// of the keyword alone.
    #[test]
    fn a_keyword_between_long_unspaced_runs_keeps_its_context() {
        let text = format!("{} needle {}", "ก".repeat(300), "ข".repeat(300));
        assert_eq!(
            snippet(&text, &query_phrases("needle")),
            format!("…{} needle {}…", "ก".repeat(95), "ข".repeat(95))
        );
    }

    #[test]
    fn a_fifty_character_word_at_the_edge_of_a_centred_snippet_is_dropped_not_split() {
        let text = format!(
            "{} {} {} needle {} {} {}",
            words("alpha", 100),
            "q".repeat(50),
            words("w", 23),
            words("w", 23),
            "z".repeat(50),
            words("omega", 100)
        );
        assert_eq!(
            snippet(&text, &query_phrases("needle")),
            format!("…{} needle {}…", words("w", 23), words("w", 23))
        );
    }

    /// The keyword index breaks a word at every mark that is not a Latin
    /// style accent, so a Devanagari query matches inside a longer word.
    #[test]
    fn a_match_inside_a_longer_devanagari_word_is_found_as_the_index_finds_it() {
        let text = format!(
            "{} हिंदीभाषी लोग {}",
            words("alpha", 100),
            words("omega", 100)
        );
        assert!(snippet(&text, &query_phrases("हिंदी")).contains(" हिंदीभाषी लोग "));
    }

    #[test]
    fn a_centred_snippet_never_opens_on_whitespace() {
        let text = format!(
            "{}{}needle {}",
            "x".repeat(300),
            " ".repeat(150),
            words("y", 200)
        );
        assert_eq!(
            snippet(&text, &query_phrases("needle")),
            format!("…needle {}…", words("y", 48))
        );
    }

    #[test]
    fn an_occurrence_too_long_to_show_gives_way_to_a_later_one_that_fits() {
        let text = format!(
            "{} well{}known {} well-known {}",
            words("alpha", 100),
            "-".repeat(250),
            words("alpha", 100),
            words("omega", 100)
        );
        assert!(snippet(&text, &query_phrases("well-known")).contains(" well-known "));
    }

    #[test]
    fn a_keyword_near_the_end_keeps_the_end_and_drops_only_the_start() {
        let text = format!("{} needle tail", words("alpha", 100));
        assert_eq!(
            snippet(&text, &query_phrases("needle")),
            format!("…{} needle tail", words("alpha", 31))
        );
    }

    #[test]
    fn a_match_ignores_case_and_accents_as_the_keyword_index_does() {
        let text = format!("{} Résumé {}", words("alpha", 100), words("omega", 100));
        assert!(snippet(&text, &query_phrases("resume")).contains(" Résumé "));
        // The same word stored decomposed, as macOS writes file content.
        let decomposed = format!(
            "{} Re\u{301}sume\u{301} {}",
            words("alpha", 100),
            words("omega", 100)
        );
        assert!(snippet(&decomposed, &query_phrases("RÉSUMÉ")).contains(" Re\u{301}sume\u{301} "));
    }

    #[test]
    fn a_hyphenated_query_word_is_found_as_the_phrase_the_index_matched() {
        // The lone `well` at the start is not what the keyword index matched.
        let text = format!(
            "well {} well-known {}",
            words("alpha", 100),
            words("omega", 100)
        );
        assert!(snippet(&text, &query_phrases("well-known")).contains(" well-known "));
    }

    #[test]
    fn a_query_word_absent_from_the_chunk_falls_back_to_its_start() {
        let text = words("word", 100);
        assert_eq!(
            snippet(&text, &query_phrases("needle")),
            format!("{}…", words("word", 40))
        );
    }

    #[test]
    fn text_written_without_spaces_is_cut_between_characters() {
        let text = "漢".repeat(300);
        assert_eq!(snippet(&text, &[]), format!("{}…", "漢".repeat(199)));

        let centred = format!("{}。needle。{}", "漢".repeat(300), "字".repeat(300));
        assert_eq!(
            snippet(&centred, &query_phrases("needle")),
            format!("…{}。needle。{}…", "漢".repeat(95), "字".repeat(95))
        );
    }

    #[test]
    fn a_cut_never_separates_an_emoji_from_its_joiners_or_a_letter_from_its_accent() {
        // One family is five characters: three people and two joiners.
        let family = "👨\u{200D}👩\u{200D}👧";
        assert_eq!(
            snippet(&family.repeat(100), &[]),
            format!("{}…", family.repeat(39))
        );
        // `e` plus a combining acute, twice over the window's edge.
        let accented = "e\u{301}".repeat(150);
        assert_eq!(
            snippet(&accented, &[]),
            format!("{}…", "e\u{301}".repeat(99))
        );
        // Hangul stored decomposed: one syllable is three conjoining jamo.
        let syllable = "\u{1112}\u{1161}\u{11AB}";
        assert_eq!(
            snippet(&syllable.repeat(100), &[]),
            format!("{}…", syllable.repeat(66))
        );
        // The flag of England: a black flag and six tag characters.
        let england = "🏴\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}";
        assert_eq!(
            snippet(&england.repeat(50), &[]),
            format!("{}…", england.repeat(28))
        );
        let flags = "🇫🇷".repeat(150);
        assert_eq!(snippet(&flags, &[]), format!("{}…", "🇫🇷".repeat(99)));
    }

    #[test]
    fn only_a_keyword_response_places_its_snippets_on_the_query() {
        let text = format!("{} needle {}", words("alpha", 100), words("omega", 100));
        let response = |mode| VaultSearchResponse {
            mode,
            results: vec![VaultSearchResult {
                vault_id: VaultId::generate().expect("vault id"),
                chunk_id: 7,
                note_slug: "note".to_owned(),
                note_title: "Note".to_owned(),
                note_path: "Folder/Note".to_owned(),
                heading_path: Some("Note > Part".to_owned()),
                content: text.clone(),
                score: 0.5,
                layer: Some("archive".to_owned()),
                outbound_links: Vec::new(),
                metadata: crate::vault::NoteMetadata::default(),
            }],
        };

        let keyword =
            CompactSearchResponse::from_full(response(SearchResponseMode::Keyword), "needle");
        assert!(keyword.results[0].snippet.contains(" needle "));
        let semantic =
            CompactSearchResponse::from_full(response(SearchResponseMode::Semantic), "needle");
        assert_eq!(
            semantic.results[0].snippet,
            format!("{}…", words("alpha", 33))
        );

        let hit = serde_json::to_value(&keyword.results[0]).expect("serializes");
        let mut fields = hit
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        fields.sort_unstable();
        assert_eq!(
            fields,
            [
                "heading_path",
                "layer",
                "note_path",
                "note_slug",
                "note_title",
                "score",
                "snippet",
                "vault_id",
            ]
        );
        assert_eq!(hit["heading_path"], "Note > Part");
        assert_eq!(hit["layer"], "archive");
        assert_eq!(hit["score"], 0.5);
    }
}
