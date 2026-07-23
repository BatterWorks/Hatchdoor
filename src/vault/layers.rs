//! Layer markers: parsing `.hatchdoor-layer` files and resolving which layer a
//! vault path belongs to. Pure logic — the walk policy lives in `index.rs`.

use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

pub const MARKER_FILE_NAME: &str = ".hatchdoor-layer";

/// Names that select surfaces rather than name a layer. A marker may not claim
/// one: `all` would let a folder rename itself into a wildcard, and `noise` is
/// deliberately not expressible in-vault.
pub const RESERVED_LAYER_NAMES: [&str; 4] = ["default", "all", "noise", "none"];

const MAX_NAME_CHARS: usize = 32;

/// NFKC → trim → lowercase → spaces to `-`, then validate against a whitelist
/// of alphanumerics plus `-`.
///
/// Names are Unicode, not ASCII: `sources-privées` and `資料` are as valid as
/// `sources`. The whitelist is what makes that safe. Layer names reach an MCP
/// tool schema that agents read and a URL query parameter, so the characters
/// that matter are the ones that can make two names *look* identical — and
/// none of them are alphanumeric. Zero-width spaces and joiners, bidirectional
/// overrides (U+202E and friends), control characters, punctuation and emoji
/// are all rejected by the whitelist without needing a rule of their own.
///
/// NFKC additionally folds compatibility variants, so full-width `ＳＯＵＲＣＥＳ`
/// becomes `sources`, and composed and decomposed spellings of the same
/// accented name normalize to one layer rather than two visually identical
/// ones.
///
/// What remains is homoglyph confusion between scripts — Cyrillic `а` against
/// Latin `a`. Catching that needs a UTS #39 mixed-script check and a
/// dependency to go with it; it is deliberately not done here, because the
/// hostile path (an agent planting a marker) is closed separately by write
/// tools refusing to write `.hatchdoor-layer`, leaving only a single-user
/// vault owner confusing themselves.
pub fn normalize_layer_name(raw: &str) -> Result<String, String> {
    let normalized: String = raw.nfkc().collect::<String>().trim().to_lowercase();
    let candidate: String = normalized
        .chars()
        .map(|c| if c == ' ' { '-' } else { c })
        .collect();

    if candidate.is_empty() {
        return Err("layer name is empty".to_string());
    }
    if candidate.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "layer name '{candidate}' exceeds {MAX_NAME_CHARS} characters"
        ));
    }
    if RESERVED_LAYER_NAMES.contains(&candidate.as_str()) {
        return Err(format!("layer name '{candidate}' is reserved"));
    }

    let mut chars = candidate.chars();
    let first = chars.next().expect("non-empty checked above");
    if !first.is_alphanumeric() {
        return Err(format!(
            "layer name '{candidate}' must start with a letter or digit"
        ));
    }
    if !chars.all(|c| c.is_alphanumeric() || c == '-') {
        return Err(format!(
            "layer name '{candidate}' may contain only letters, digits and '-'"
        ));
    }

    Ok(candidate)
}

const MAX_DESCRIPTION_CHARS: usize = 500;

/// Test whether a character is a Unicode format character (Cf category) that can
/// visually reorder or hide text in schemas. Covers the specific ranges that
/// matter for visual spoofing:
/// - U+200B–U+200F: zero-width space, ZWNJ, ZWJ, LRM, RLM
/// - U+202A–U+202E: legacy bidi embedding/override controls (LRE, RLE, PDF, LRO, RLO)
/// - U+2060–U+2064: word joiner and invisible operators
/// - U+2066–U+2069: directional isolates (LDI, RDI, FSI, PDI)
/// - U+FEFF: byte-order mark / zero-width no-break space
fn is_format_character(c: char) -> bool {
    matches!(
        c as u32,
        0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x2064 | 0x2066..=0x2069 | 0xfeff
    )
}

/// What a `.hatchdoor-layer` file declares about its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerDecl {
    /// Re-include this subtree onto the default surface, overriding an
    /// inherited layer. Not a layer: never enumerated, always reports
    /// `layer: null`.
    Default,
    Named {
        name: String,
        description: Option<String>,
    },
}

/// A bare word is itself a valid YAML scalar document, so one parser handles
/// both the `sources` and the `name: sources` forms. Unknown keys are ignored
/// (serde's default) so the format can grow.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawMarker {
    Bare(String),
    Full {
        name: String,
        #[serde(default)]
        description: Option<String>,
    },
}

pub fn parse_marker(contents: &str) -> Result<LayerDecl, String> {
    let raw: RawMarker =
        serde_yaml::from_str(contents).map_err(|e| format!("malformed {MARKER_FILE_NAME}: {e}"))?;

    let (name, description) = match raw {
        RawMarker::Bare(name) => (name, None),
        RawMarker::Full { name, description } => (name, description),
    };

    if name.nfkc().collect::<String>().trim().to_lowercase() == "default" {
        return Ok(LayerDecl::Default);
    }

    Ok(LayerDecl::Named {
        name: normalize_layer_name(&name)?,
        description: description.as_deref().map(sanitize_description),
    })
}

/// Descriptions reach the MCP tool schema every agent reads, so they are
/// treated as untrusted vault content: control characters and format characters
/// stripped, newlines collapsed, length capped.
fn sanitize_description(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || is_format_character(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(MAX_DESCRIPTION_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_layer_name_slugifies() {
        assert_eq!(normalize_layer_name("Sources").expect("valid"), "sources");
        assert_eq!(
            normalize_layer_name("  My Sources  ").expect("valid"),
            "my-sources"
        );
    }

    #[test]
    fn normalize_layer_name_rejects_reserved_and_malformed() {
        for reserved in ["default", "all", "noise", "none", "DEFAULT"] {
            assert!(
                normalize_layer_name(reserved).is_err(),
                "{reserved} must be reserved"
            );
        }
        assert!(normalize_layer_name("").is_err());
        assert!(normalize_layer_name("   ").is_err());
        assert!(normalize_layer_name("-leading").is_err());
        assert!(normalize_layer_name("has_underscore").is_err());
        assert!(normalize_layer_name(&"x".repeat(33)).is_err());
    }

    #[test]
    fn normalize_layer_name_applies_nfkc_before_validating() {
        // NFKC folds compatibility variants, so a full-width name is usable
        // rather than mysteriously rejected.
        assert_eq!(
            normalize_layer_name("\u{ff33}\u{ff2f}\u{ff35}\u{ff32}\u{ff23}\u{ff25}\u{ff33}")
                .expect("valid"),
            "sources"
        );
        // It also collapses composed and decomposed spellings of the same
        // accented name into one layer, rather than two that look identical.
        assert_eq!(
            normalize_layer_name("sourc\u{0065}\u{0301}s").expect("valid"),
            normalize_layer_name("sourc\u{00e9}s").expect("valid")
        );
    }

    #[test]
    fn normalize_layer_name_accepts_non_ascii_scripts() {
        // A vault is not required to be English. Names are Unicode.
        assert_eq!(
            normalize_layer_name("Sources-Privées").expect("valid"),
            "sources-privées"
        );
        assert_eq!(normalize_layer_name("資料").expect("valid"), "資料");
        assert_eq!(
            normalize_layer_name("Материалы").expect("valid"),
            "материалы"
        );
    }

    #[test]
    fn normalize_layer_name_rejects_invisible_and_directional_characters() {
        // These are the characters that let two names render identically or
        // reorder the tool schema an agent reads. None are alphanumeric, so
        // the whitelist refuses them without a rule of their own.
        assert!(
            normalize_layer_name("sour\u{200b}ces").is_err(),
            "zero-width space"
        );
        assert!(
            normalize_layer_name("sour\u{200d}ces").is_err(),
            "zero-width joiner"
        );
        assert!(
            normalize_layer_name("sour\u{202e}ces").is_err(),
            "right-to-left override"
        );
        assert!(normalize_layer_name("sources\u{1f600}").is_err(), "emoji");
        assert!(normalize_layer_name("sources/raw").is_err(), "punctuation");
    }

    #[test]
    fn normalize_layer_name_caps_length_in_characters_not_bytes() {
        // `é` is one character and two bytes: a byte-based cap would reject a
        // name that is well under the limit.
        let thirty_two_chars = "é".repeat(32);
        assert_eq!(thirty_two_chars.len(), 64, "precondition: 64 bytes");
        assert_eq!(
            normalize_layer_name(&thirty_two_chars)
                .expect("valid")
                .chars()
                .count(),
            32
        );
        assert!(normalize_layer_name(&"é".repeat(33)).is_err());
    }

    #[test]
    fn parse_marker_accepts_bare_scalar_form() {
        assert_eq!(
            parse_marker("sources\n").expect("valid"),
            LayerDecl::Named {
                name: "sources".to_string(),
                description: None
            }
        );
    }

    #[test]
    fn parse_marker_accepts_mapping_form_and_ignores_unknown_keys() {
        let marker = "name: sources\ndescription: Ground truth.\nfuture_key: 1\n";
        assert_eq!(
            parse_marker(marker).expect("valid"),
            LayerDecl::Named {
                name: "sources".to_string(),
                description: Some("Ground truth.".to_string())
            }
        );
    }

    #[test]
    fn parse_marker_recognises_default_reinclude() {
        assert_eq!(parse_marker("default").expect("valid"), LayerDecl::Default);
        assert_eq!(
            parse_marker("name: default\n").expect("valid"),
            LayerDecl::Default
        );
    }

    #[test]
    fn parse_marker_rejects_malformed_and_reserved() {
        assert!(parse_marker("").is_err());
        assert!(parse_marker("- a\n- b\n").is_err());
        assert!(parse_marker("name: all\n").is_err());
    }

    #[test]
    fn parse_marker_sanitizes_and_caps_description() {
        // Descriptions are vault-authored text rendered into the MCP tool schema:
        // control characters and newlines must not corrupt it, and length is capped.
        let marker = format!(
            "name: sources\ndescription: \"line one\\nline\\ttwo\\u0007 {}\"\n",
            "x".repeat(600)
        );
        let LayerDecl::Named { description, .. } = parse_marker(&marker).expect("valid") else {
            panic!("expected a named layer");
        };
        let description = description.expect("description present");
        assert!(!description.contains('\n'));
        assert!(!description.contains('\u{0007}'));
        assert!(description.starts_with("line one line two"));
        assert_eq!(description.chars().count(), 500);
    }

    #[test]
    fn sanitize_description_strips_format_characters() {
        // Format characters (Cf category) can visually reorder or hide text in
        // schemas that agents read. They must be removed along with control
        // characters. Test the specific ranges that matter for visual spoofing.

        // Zero-width space U+200B, ZWNJ U+200D, ZWJ U+200C, LRM U+200E, RLM U+200F
        let with_zwsp = "normal\u{200b}text";
        assert!(!sanitize_description(with_zwsp).contains('\u{200b}'));

        let with_zwnj = "normal\u{200d}text";
        assert!(!sanitize_description(with_zwnj).contains('\u{200d}'));

        let with_zwj = "normal\u{200c}text";
        assert!(!sanitize_description(with_zwj).contains('\u{200c}'));

        let with_lrm = "normal\u{200e}text";
        assert!(!sanitize_description(with_lrm).contains('\u{200e}'));

        let with_rlm = "normal\u{200f}text";
        assert!(!sanitize_description(with_rlm).contains('\u{200f}'));

        // Legacy bidi embedding/override controls: U+202A–U+202E
        let with_lre = "normal\u{202a}text";
        assert!(!sanitize_description(with_lre).contains('\u{202a}'));

        let with_rle = "normal\u{202b}text";
        assert!(!sanitize_description(with_rle).contains('\u{202b}'));

        let with_pdf = "normal\u{202c}text";
        assert!(!sanitize_description(with_pdf).contains('\u{202c}'));

        let with_lro = "normal\u{202d}text";
        assert!(!sanitize_description(with_lro).contains('\u{202d}'));

        let with_rlo = "normal\u{202e}text";
        assert!(!sanitize_description(with_rlo).contains('\u{202e}'));

        // Word joiner and invisible operators: U+2060–U+2064
        let with_wj = "normal\u{2060}text";
        assert!(!sanitize_description(with_wj).contains('\u{2060}'));

        let with_ifm = "normal\u{2061}text";
        assert!(!sanitize_description(with_ifm).contains('\u{2061}'));

        let with_it = "normal\u{2062}text";
        assert!(!sanitize_description(with_it).contains('\u{2062}'));

        let with_is = "normal\u{2063}text";
        assert!(!sanitize_description(with_is).contains('\u{2063}'));

        let with_ip = "normal\u{2064}text";
        assert!(!sanitize_description(with_ip).contains('\u{2064}'));

        // Directional isolates: U+2066–U+2069
        let with_ldi = "normal\u{2066}text";
        assert!(!sanitize_description(with_ldi).contains('\u{2066}'));

        let with_rdi = "normal\u{2067}text";
        assert!(!sanitize_description(with_rdi).contains('\u{2067}'));

        let with_fsi = "normal\u{2068}text";
        assert!(!sanitize_description(with_fsi).contains('\u{2068}'));

        let with_pdi = "normal\u{2069}text";
        assert!(!sanitize_description(with_pdi).contains('\u{2069}'));

        // Byte-order mark / zero-width no-break space: U+FEFF
        let with_bom = "normal\u{feff}text";
        assert!(!sanitize_description(with_bom).contains('\u{feff}'));

        // Verify ordinary text still survives
        assert_eq!(
            sanitize_description("A normal description"),
            "A normal description"
        );
        assert_eq!(
            sanitize_description("with    multiple  spaces"),
            "with multiple spaces"
        );
        assert_eq!(
            sanitize_description("unicode: café, 資料, Материалы"),
            "unicode: café, 資料, Материалы"
        );
    }
}
