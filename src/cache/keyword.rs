//! How the keyword index reads Chinese, Japanese, Korean and Thai (#634).
//!
//! FTS5's `unicode61` tokenizer splits only at separators, and these scripts
//! are written without spaces between words, so a whole clause became one
//! token and a word inside it never matched. Text in these scripts is
//! rewritten before it reaches the tokenizer: every run of them becomes
//! overlapping two-character tokens, followed by the run's last character on
//! its own. `本文です` is indexed as `本文 文で です す`. Every character then
//! starts a token, so a one-character query is a prefix query.
//!
//! A query term gets the same rewrite and is matched as a phrase, so `本文`
//! matches `…混ざった本文です` and `日本語` matches `日本 本語`. Other scripts
//! pass through unchanged. Text and queries are both NFC-normalised, so a
//! decomposed Hangul syllable matches its composed form.

use unicode_normalization::{
    IsNormalized, UnicodeNormalization, char::is_combining_mark, is_nfc_quick,
};

/// The Thai marks the keyword tokenizer keeps inside a token. `unicode61`
/// counts only letters and digits as token characters, and these vowel and
/// tone marks are combining marks, so without them a Thai pair such as `ที่`
/// would split around its marks.
pub(crate) const THAI_MARKS: &str = "\u{0E31}\u{0E34}\u{0E35}\u{0E36}\u{0E37}\u{0E38}\u{0E39}\u{0E3A}\
     \u{0E47}\u{0E48}\u{0E49}\u{0E4A}\u{0E4B}\u{0E4C}\u{0E4D}\u{0E4E}";

/// Whether `ch` belongs to a script the keyword index splits into pairs:
/// CJK ideographs, kana, Hangul and Thai. Punctuation in those blocks, such
/// as `、` or `・`, is not included and separates runs as any other does, and
/// so is a combining mark other than [`THAI_MARKS`], such as a kana voicing
/// mark with no precomposed form: the tokenizer splits at those too.
pub(crate) fn is_bigram_char(ch: char) -> bool {
    let in_block = matches!(
        u32::from(ch),
        0x0E00..=0x0E7F // Thai
            | 0x1100..=0x11FF // Hangul Jamo
            | 0x2E80..=0x2FDF // CJK radicals
            | 0x3005..=0x3007 // 々 〆 〇
            | 0x3021..=0x3029 // Hangzhou numerals
            | 0x3031..=0x3035 // kana repeat marks
            | 0x303B // 〻
            | 0x3040..=0x30FF // Hiragana and Katakana
            | 0x3130..=0x318F // Hangul compatibility jamo
            | 0x31F0..=0x31FF // Katakana phonetic extensions
            | 0x3400..=0x4DBF // CJK extension A
            | 0x4E00..=0x9FFF // CJK unified ideographs
            | 0xA960..=0xA97F // Hangul Jamo extended A
            | 0xAC00..=0xD7FF // Hangul syllables and Jamo extended B
            | 0xF900..=0xFAFF // CJK compatibility ideographs
            | 0xFF66..=0xFF9F // halfwidth Katakana
            | 0xFFA0..=0xFFDC // halfwidth Hangul
            | 0x20000..=0x3FFFF // CJK extensions B and later
    );
    in_block && (THAI_MARKS.contains(ch) || (ch.is_alphanumeric() && !is_combining_mark(ch)))
}

/// The most characters of CJK, kana, Hangul or Thai one query word keeps.
/// Every character beyond the first is one more token the phrase must look
/// up, so an unbroken 8 KB run would cost thousands of lookups; 24
/// characters already pin a phrase down.
const MAX_QUERY_RUN_CHARS: usize = 24;

/// `term`, cut after [`MAX_QUERY_RUN_CHARS`] characters when it holds CJK,
/// kana, Hangul or Thai, never between a character and its marks.
pub(crate) fn bounded_query_term(term: &str) -> &str {
    if !term.chars().any(is_bigram_char) {
        return term;
    }
    let mut kept = 0;
    for (at, ch) in term.char_indices() {
        if is_combining_mark(ch) || is_trailing_jamo(ch) {
            continue;
        }
        if kept == MAX_QUERY_RUN_CHARS {
            return &term[..at];
        }
        kept += 1;
    }
    term
}

/// A Hangul vowel or final consonant jamo, which renders as one syllable with
/// the jamo or syllable before it.
pub(crate) fn is_trailing_jamo(ch: char) -> bool {
    matches!(u32::from(ch), 0x1160..=0x11FF | 0xD7B0..=0xD7FF)
}

fn is_hangul(ch: char) -> bool {
    matches!(
        u32::from(ch),
        0x1100..=0x11FF | 0xA960..=0xA97F | 0xAC00..=0xD7A3 | 0xD7B0..=0xD7FF
    )
}

/// One token the keyword index makes from a run: a pair of neighbouring
/// characters, or the run's last character alone. Character indices, end
/// exclusive.
pub(crate) struct RunToken {
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// For the run's lone last character, the run's length in characters.
    pub(crate) run_len: Option<usize>,
}

/// The tokens of the run of [`is_bigram_char`] characters starting at
/// `start`, in order: every overlapping pair, then the last character alone.
/// The last token's `end` is where the run stops.
pub(crate) fn run_tokens(chars: &[char], start: usize) -> Vec<RunToken> {
    let units = run_units(chars, start);
    let mut tokens = units
        .windows(2)
        .map(|pair| RunToken {
            start: pair[0].0,
            end: pair[1].1,
            run_len: None,
        })
        .collect::<Vec<_>>();
    if let Some(&(start, end)) = units.last() {
        tokens.push(RunToken {
            start,
            end,
            run_len: Some(units.len()),
        });
    }
    tokens
}

/// The units of the run of [`is_bigram_char`] characters starting at
/// `start`, as `(start, end)` character indices, end exclusive. A unit is one
/// character and the marks, or Hangul jamo, that render with it, so a pair
/// never begins with a mark and a decomposed syllable counts once.
fn run_units(chars: &[char], start: usize) -> Vec<(usize, usize)> {
    let mut units: Vec<(usize, usize)> = Vec::new();
    let mut index = start;
    while index < chars.len() && is_bigram_char(chars[index]) {
        let ch = chars[index];
        let joins = index > start
            && (is_combining_mark(ch) || (is_trailing_jamo(ch) && is_hangul(chars[index - 1])));
        match units.last_mut() {
            Some(unit) if joins => unit.1 = index + 1,
            _ => units.push((index, index + 1)),
        }
        index += 1;
    }
    units
}

/// The text the keyword index tokenizes for `content`, or `None` when that
/// is `content` itself, which is true of any NFC text with no CJK, kana,
/// Hangul or Thai in it.
pub(crate) fn keyword_index_text(content: &str) -> Option<String> {
    if is_nfc_quick(content.chars()) == IsNormalized::Yes && !content.chars().any(is_bigram_char) {
        return None;
    }
    let normalised = content.nfc().collect::<String>();
    let text = split_runs(&normalised, false).0;
    (text != content).then_some(text)
}

/// One query term as the FTS5 phrase that finds it, and whether its last
/// token is a prefix. Run through the same rewrite as the indexed text, with
/// two differences at the end of the term, where the text may go on: a run
/// of two or more characters leaves out its lone last character, and a
/// one-character run becomes a prefix of the pair it starts.
pub(crate) fn keyword_query_phrase(term: &str) -> (String, bool) {
    let normalised = term.nfc().collect::<String>();
    if !normalised.chars().any(is_bigram_char) {
        return (normalised, false);
    }
    let (text, prefix) = split_runs(&normalised, true);
    (
        text.split_whitespace().collect::<Vec<_>>().join(" "),
        prefix,
    )
}

fn split_runs(text: &str, query: bool) -> (String, bool) {
    let chars = text.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(text.len() * 3);
    let mut prefix = false;
    let mut index = 0;
    while index < chars.len() {
        if !is_bigram_char(chars[index]) {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        let tokens = run_tokens(&chars, index);
        let end = tokens.last().map_or(index + 1, |token| token.end);
        let ends_term = query && !chars[end..].iter().any(|ch| ch.is_alphanumeric());
        out.push(' ');
        for token in &tokens {
            let lone_last_of_longer_run = token.run_len.is_some_and(|len| len > 1);
            if ends_term && lone_last_of_longer_run {
                continue;
            }
            out.extend(&chars[token.start..token.end]);
            out.push(' ');
        }
        prefix = ends_term && tokens.len() == 1;
        index = end;
    }
    (out, prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_becomes_overlapping_pairs_and_its_last_character() {
        assert_eq!(
            keyword_index_text("漢字が混ざった本文です。").as_deref(),
            Some(" 漢字 字が が混 混ざ ざっ った た本 本文 文で です す 。")
        );
        assert_eq!(keyword_index_text("本").as_deref(), Some(" 本 "));
    }

    #[test]
    fn other_scripts_are_left_alone_and_nfc_text_is_not_copied() {
        for text in ["plain words", "Кириллица, ё", "ملاحظة عربية", "café"] {
            assert_eq!(keyword_index_text(text), None, "{text}");
        }
        assert_eq!(
            keyword_index_text("iPhone日本で").as_deref(),
            Some("iPhone 日本 本で で ")
        );
    }

    #[test]
    fn text_is_normalised_to_nfc() {
        let decomposed = "cafe\u{301}".to_string();
        assert_eq!(keyword_index_text(&decomposed).as_deref(), Some("café"));
        let hangul = "한국어".nfd().collect::<String>();
        assert_eq!(
            keyword_index_text(&hangul),
            keyword_index_text("한국어"),
            "decomposed and composed Hangul index alike"
        );
        assert_eq!(
            keyword_index_text("한국어").as_deref(),
            Some(" 한국 국어 어 ")
        );
    }

    #[test]
    fn thai_marks_stay_with_their_letter() {
        assert_eq!(keyword_index_text("ที่นี่").as_deref(), Some(" ที่นี่ นี่ "));
    }

    #[test]
    fn a_decomposed_syllable_is_one_unit() {
        let chars = "한국".nfd().collect::<Vec<_>>();
        assert_eq!(chars.len(), 6);
        assert_eq!(run_units(&chars, 0), [(0, 3), (3, 6)]);
        let tokens = run_tokens(&chars, 0);
        let spans = tokens
            .iter()
            .map(|token| (token.start, token.end, token.run_len))
            .collect::<Vec<_>>();
        assert_eq!(spans, [(0, 6, None), (3, 6, Some(2))]);
    }

    #[test]
    fn query_phrases_match_the_indexed_pairs() {
        assert_eq!(keyword_query_phrase("本文"), ("本文".into(), false));
        assert_eq!(keyword_query_phrase("日本語"), ("日本 本語".into(), false));
        assert_eq!(keyword_query_phrase("本"), ("本".into(), true));
        assert_eq!(keyword_query_phrase("x日"), ("x 日".into(), true));
        // A run followed by more text keeps its last character, as the index does.
        assert_eq!(
            keyword_query_phrase("日本xyz"),
            ("日本 本 xyz".into(), false)
        );
        assert_eq!(keyword_query_phrase("Café"), ("Café".into(), false));
        assert_eq!(
            keyword_query_phrase(&"한국어".nfd().collect::<String>()),
            ("한국 국어".into(), false)
        );
    }

    #[test]
    fn a_long_query_run_is_cut_at_a_character_boundary() {
        let long = "漢".repeat(40);
        assert_eq!(bounded_query_term(&long), "漢".repeat(24));
        assert_eq!(bounded_query_term(&"x".repeat(40)), "x".repeat(40));
        let thai = "ที่".repeat(30);
        assert_eq!(
            bounded_query_term(&thai),
            "ที่".repeat(24),
            "a mark stays with its letter"
        );
    }

    #[test]
    fn a_kana_voicing_mark_with_no_precomposed_form_separates_runs() {
        assert_eq!(
            keyword_index_text("カ\u{309A}キ").as_deref(),
            Some(" カ \u{309A} キ ")
        );
    }

    #[test]
    fn punctuation_in_cjk_blocks_separates_runs() {
        assert!(!is_bigram_char('、'));
        assert!(!is_bigram_char('・'));
        assert!(!is_bigram_char('。'));
        assert!(is_bigram_char('ー'));
        assert!(is_bigram_char('々'));
    }
}
