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

/// NFKC → trim → lowercase → spaces to `-`, then validate. Unicode
/// normalization is specified so NFC/NFD variants cannot produce two visually
/// identical layers.
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
    if !first.is_ascii_alphanumeric() {
        return Err(format!(
            "layer name '{candidate}' must start with a letter or digit"
        ));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(format!(
            "layer name '{candidate}' may contain only letters, digits and '-'"
        ));
    }

    Ok(candidate)
}

const MAX_DESCRIPTION_CHARS: usize = 500;

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
/// treated as untrusted vault content: control characters stripped, newlines
/// collapsed, length capped.
fn sanitize_description(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
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
        // Names are ASCII by contract. NFKC earns its place two ways.
        // First, it folds compatibility variants into ASCII, so a full-width
        // name is usable rather than mysteriously rejected.
        assert_eq!(
            normalize_layer_name("\u{ff33}\u{ff2f}\u{ff35}\u{ff32}\u{ff23}\u{ff25}\u{ff33}")
                .expect("valid"),
            "sources"
        );
        // Second, it makes rejection deterministic: an accented name is
        // refused identically whether composed or decomposed, so the two can
        // never become two visually identical layers.
        assert!(normalize_layer_name("sourc\u{0065}\u{0301}s").is_err());
        assert!(normalize_layer_name("sourc\u{00e9}s").is_err());
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
}
