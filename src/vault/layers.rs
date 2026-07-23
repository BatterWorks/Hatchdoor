//! Layer markers: parsing `.hatchdoor-layer` files and resolving which layer a
//! vault path belongs to. Pure logic — the walk policy lives in `index.rs`.

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
}
