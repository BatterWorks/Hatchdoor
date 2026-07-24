//! Layer markers: parsing `.hatchdoor-layer` files and resolving which layer a
//! vault path belongs to. This module owns the marker-collection walk (a separate,
//! independent traversal from the noise-pruning content walk in `index.rs`), because
//! a marker inside a noise-pruned directory must still be collected for portability.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;

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

/// Which layer each marked directory declares, keyed by vault-relative
/// directory path (`""` is the vault root). Resolution is longest-prefix.
#[derive(Debug, Clone, Default)]
pub struct LayerMap {
    by_dir: BTreeMap<String, LayerDecl>,
    descriptions: BTreeMap<String, String>,
}

impl LayerMap {
    /// Walks the tree collecting markers. Deliberately independent of noise
    /// pruning: a marker inside a directory a noise pattern would prune is
    /// still collected, because per-deployment noise silently deleting a layer
    /// would contradict the portability the marker mechanism exists for.
    /// Directory symlinks are not followed.
    pub fn collect(root: &Path) -> Result<Self, String> {
        let mut by_dir: BTreeMap<String, LayerDecl> = BTreeMap::new();
        // name -> (marker path, description); lexicographically-first wins.
        let mut chosen: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();

        let mut marker_paths: Vec<(String, String)> = Vec::new();
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry.map_err(|e| format!("vault walk failed: {e}"))?;
            if !entry.file_type().is_file() || entry.file_name().to_str() != Some(MARKER_FILE_NAME)
            {
                continue;
            }
            let relative_dir = entry
                .path()
                .parent()
                .and_then(|parent| parent.strip_prefix(root).ok())
                .and_then(|relative| relative.to_str())
                .map(|relative| relative.replace('\\', "/"))
                .ok_or_else(|| "marker path outside vault root".to_string())?;
            let display = if relative_dir.is_empty() {
                MARKER_FILE_NAME.to_string()
            } else {
                format!("{relative_dir}/{MARKER_FILE_NAME}")
            };
            marker_paths.push((relative_dir, display));
        }
        marker_paths.sort();

        for (relative_dir, display) in marker_paths {
            let contents = fs::read_to_string(root.join(&display))
                .map_err(|e| format!("could not read {display}: {e}"))?;
            let decl = parse_marker(&contents).map_err(|e| format!("{display}: {e}"))?;

            if relative_dir.is_empty() && !matches!(decl, LayerDecl::Default) {
                return Err(format!(
                    "{display}: a named layer marker at the vault root would demote the \
                     entire vault, leaving the default surface empty"
                ));
            }

            if let LayerDecl::Named { name, description } = &decl {
                let entry = chosen
                    .entry(name.clone())
                    .or_insert_with(|| (display.clone(), description.clone()));
                // When multiple markers declare the same layer name, the
                // lexicographically-first marker's path wins. However, if the first
                // marker has no description and a later one does, the later description
                // backfills. This is deliberate: a description beats none, and picking
                // the first *non-empty* description in lexicographic order is still
                // fully deterministic and more useful than silently discarding a
                // provided description.
                if entry.1.is_none() {
                    entry.1 = description.clone();
                }
            }

            by_dir.insert(relative_dir, decl);
        }

        let descriptions = chosen
            .into_iter()
            .filter_map(|(name, (_, description))| description.map(|d| (name, d)))
            .collect();

        Ok(Self {
            by_dir,
            descriptions,
        })
    }

    /// `None` means the default surface.
    pub fn layer_for(&self, relative_path: &str) -> Option<&str> {
        if self.by_dir.is_empty() {
            return None;
        }

        let mut best: Option<&LayerDecl> = None;
        let mut best_len = 0usize;
        for (dir, decl) in &self.by_dir {
            let matches = dir.is_empty()
                || relative_path == dir
                || relative_path.starts_with(&format!("{dir}/"));
            if matches && (dir.len() >= best_len || best.is_none()) {
                best_len = dir.len();
                best = Some(decl);
            }
        }

        match best {
            Some(LayerDecl::Named { name, .. }) => Some(name.as_str()),
            _ => None,
        }
    }

    pub fn layer_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .by_dir
            .values()
            .filter_map(|decl| match decl {
                LayerDecl::Named { name, .. } => Some(name.clone()),
                LayerDecl::Default => None,
            })
            .collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn description(&self, name: &str) -> Option<&str> {
        self.descriptions.get(name).map(String::as_str)
    }

    /// Marker directories, for diagnostics and (in phase 2) the marker-set hash.
    pub fn marker_paths(&self) -> Vec<String> {
        self.by_dir.keys().cloned().collect()
    }

    /// Diagnostics helper: layer names whose markers declare more than one
    /// distinct description. `collect` resolves these silently (the first
    /// non-empty description in lexicographic order wins); the diagnostics
    /// surface reports them so an operator can reconcile the markers. Malformed
    /// markers are skipped here — they surface as an index build failure instead.
    pub fn description_conflicts(root: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
        let mut by_name: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry.map_err(|e| format!("vault walk failed: {e}"))?;
            if !entry.file_type().is_file() || entry.file_name().to_str() != Some(MARKER_FILE_NAME)
            {
                continue;
            }
            let Ok(contents) = fs::read_to_string(entry.path()) else {
                continue;
            };
            if let Ok(LayerDecl::Named {
                name,
                description: Some(description),
            }) = parse_marker(&contents)
            {
                by_name.entry(name).or_default().insert(description);
            }
        }
        Ok(by_name
            .into_iter()
            .filter(|(_, descriptions)| descriptions.len() > 1)
            .map(|(name, descriptions)| (name, descriptions.into_iter().collect()))
            .collect())
    }

    /// A deterministic string covering every marker's directory path *and its
    /// resolved declaration* (kind, name and description). Hashing this and
    /// comparing against the stored value is what forces a note-row refresh when
    /// the marker set changes: adding, removing, renaming a layer or editing a
    /// description all change this string, and none of them touch any note's
    /// content or mtime — so without it the incremental path would silently
    /// leave every note on its old classification.
    ///
    /// `by_dir` is a `BTreeMap`, so iteration order is stable across restarts.
    /// A record separator that cannot appear in a directory path, layer name or
    /// sanitized description (`\u{1}`) delimits fields so distinct marker sets
    /// cannot collide into one string.
    pub fn hash_input(&self) -> String {
        let mut input = String::new();
        for (dir, decl) in &self.by_dir {
            input.push_str(dir);
            input.push('\u{1}');
            match decl {
                LayerDecl::Default => input.push_str("default"),
                LayerDecl::Named { name, description } => {
                    input.push_str("named");
                    input.push('\u{1}');
                    input.push_str(name);
                    input.push('\u{1}');
                    input.push_str(description.as_deref().unwrap_or(""));
                }
            }
            input.push('\u{2}');
        }
        input
    }

    /// The resolved named-layer markers as `directory path -> layer name`,
    /// excluding `default` re-includes (which are not layers and cannot cause a
    /// silent *promotion* when they vanish). Persisted so a later reindex can
    /// detect a marker that disappeared and refuse to promote its notes.
    pub fn named_markers(&self) -> BTreeMap<String, String> {
        self.by_dir
            .iter()
            .filter_map(|(dir, decl)| match decl {
                LayerDecl::Named { name, .. } => Some((dir.clone(), name.clone())),
                LayerDecl::Default => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

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

    #[test]
    fn layer_map_resolves_nearest_marker_and_inherits() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("sources/deep")).expect("dirs");
        fs::create_dir_all(dir.path().join("wiki")).expect("dirs");
        fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");

        let map = LayerMap::collect(dir.path()).expect("collect");

        assert_eq!(map.layer_for("sources/A.md"), Some("sources"));
        assert_eq!(map.layer_for("sources/deep/B.md"), Some("sources"));
        assert_eq!(map.layer_for("wiki/C.md"), None);
        assert_eq!(map.layer_for("Top.md"), None);
    }

    #[test]
    fn layer_map_default_marker_reincludes_subtree() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("sources/curated")).expect("dirs");
        fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");
        fs::write(
            dir.path().join("sources/curated/.hatchdoor-layer"),
            "default",
        )
        .expect("marker");

        let map = LayerMap::collect(dir.path()).expect("collect");

        assert_eq!(map.layer_for("sources/A.md"), Some("sources"));
        assert_eq!(map.layer_for("sources/curated/B.md"), None);
    }

    #[test]
    fn layer_map_two_folders_may_share_one_layer_name() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("raw")).expect("dirs");
        fs::create_dir_all(dir.path().join("inbox")).expect("dirs");
        fs::write(dir.path().join("raw/.hatchdoor-layer"), "sources").expect("marker");
        fs::write(dir.path().join("inbox/.hatchdoor-layer"), "sources").expect("marker");

        let map = LayerMap::collect(dir.path()).expect("collect");

        assert_eq!(map.layer_for("raw/A.md"), Some("sources"));
        assert_eq!(map.layer_for("inbox/B.md"), Some("sources"));
        assert_eq!(map.layer_names(), vec!["sources".to_string()]);
    }

    #[test]
    fn layer_map_description_tiebreak_is_lexicographically_first_marker() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("aaa")).expect("dirs");
        fs::create_dir_all(dir.path().join("zzz")).expect("dirs");
        fs::write(
            dir.path().join("aaa/.hatchdoor-layer"),
            "name: sources\ndescription: from aaa\n",
        )
        .expect("marker");
        fs::write(
            dir.path().join("zzz/.hatchdoor-layer"),
            "name: sources\ndescription: from zzz\n",
        )
        .expect("marker");

        let map = LayerMap::collect(dir.path()).expect("collect");

        // Deterministic, so the generated tool schema cannot vary with filesystem
        // walk order between restarts.
        assert_eq!(map.description("sources"), Some("from aaa"));
    }

    #[test]
    fn layer_map_rejects_named_marker_at_vault_root() {
        let dir = tempdir().expect("temp dir");
        fs::write(dir.path().join(".hatchdoor-layer"), "sources").expect("marker");

        // Otherwise the default surface is empty, the UI is blank, and under
        // demo_mode there is no toggle to reveal anything.
        let err = LayerMap::collect(dir.path()).expect_err("root marker must fail");
        assert!(err.contains("vault root"), "unexpected error: {err}");
    }

    #[test]
    fn layer_map_reports_malformed_marker_with_its_path() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("sources")).expect("dirs");
        fs::write(dir.path().join("sources/.hatchdoor-layer"), "- a\n- b\n").expect("marker");

        let err = LayerMap::collect(dir.path()).expect_err("malformed marker must fail");
        assert!(
            err.contains("sources/.hatchdoor-layer"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn layer_map_does_not_confuse_sibling_directories_sharing_a_prefix() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("sources")).expect("dirs");
        fs::create_dir_all(dir.path().join("sources-old")).expect("dirs");
        fs::write(
            dir.path().join("sources/.hatchdoor-layer"),
            "name: primary\n",
        )
        .expect("marker");
        fs::write(
            dir.path().join("sources-old/.hatchdoor-layer"),
            "name: archive\n",
        )
        .expect("marker");

        let map = LayerMap::collect(dir.path()).expect("collect");

        // A note under sources/ resolves to the first layer
        assert_eq!(map.layer_for("sources/A.md"), Some("primary"));
        // A note under sources-old/ resolves to the second layer, not the first
        assert_eq!(map.layer_for("sources-old/B.md"), Some("archive"));
    }

    #[test]
    fn layer_map_does_not_confuse_sibling_directories_when_only_prefix_has_marker() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("sources")).expect("dirs");
        fs::create_dir_all(dir.path().join("sources-old")).expect("dirs");
        fs::write(dir.path().join("sources/.hatchdoor-layer"), "primary").expect("marker");

        let map = LayerMap::collect(dir.path()).expect("collect");

        // A note under sources/ resolves to the marker's layer
        assert_eq!(map.layer_for("sources/A.md"), Some("primary"));
        // A note under sources-old/ (which has no marker) resolves to None (default surface)
        assert_eq!(map.layer_for("sources-old/B.md"), None);
    }

    #[test]
    fn hash_input_changes_on_add_rename_and_description_edit() {
        let base = tempdir().expect("temp dir");
        fs::create_dir_all(base.path().join("sources")).expect("dirs");
        fs::write(base.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");
        let a = LayerMap::collect(base.path())
            .expect("collect")
            .hash_input();

        // Adding a second marker changes the hash input.
        fs::create_dir_all(base.path().join("archive")).expect("dirs");
        fs::write(base.path().join("archive/.hatchdoor-layer"), "archive").expect("marker");
        let b = LayerMap::collect(base.path())
            .expect("collect")
            .hash_input();
        assert_ne!(a, b, "adding a marker must change the hash input");

        // Renaming a layer changes it.
        fs::write(base.path().join("sources/.hatchdoor-layer"), "primary").expect("marker");
        let c = LayerMap::collect(base.path())
            .expect("collect")
            .hash_input();
        assert_ne!(b, c, "renaming a layer must change the hash input");

        // Editing a description changes it.
        fs::write(
            base.path().join("sources/.hatchdoor-layer"),
            "name: primary\ndescription: now with detail\n",
        )
        .expect("marker");
        let d = LayerMap::collect(base.path())
            .expect("collect")
            .hash_input();
        assert_ne!(c, d, "editing a description must change the hash input");
    }

    #[test]
    fn hash_input_is_stable_across_recollection() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("sources")).expect("dirs");
        fs::write(
            dir.path().join("sources/.hatchdoor-layer"),
            "name: sources\ndescription: ground truth\n",
        )
        .expect("marker");
        let first = LayerMap::collect(dir.path()).expect("collect").hash_input();
        let second = LayerMap::collect(dir.path()).expect("collect").hash_input();
        assert_eq!(first, second, "hash input must be deterministic");
    }

    #[test]
    fn named_markers_excludes_default_reincludes() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("sources/curated")).expect("dirs");
        fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");
        fs::write(
            dir.path().join("sources/curated/.hatchdoor-layer"),
            "default",
        )
        .expect("marker");

        let markers = LayerMap::collect(dir.path())
            .expect("collect")
            .named_markers();
        assert_eq!(markers.get("sources").map(String::as_str), Some("sources"));
        assert!(
            !markers.contains_key("sources/curated"),
            "a default re-include is not a named marker"
        );
    }

    #[test]
    fn layer_map_description_backfills_when_the_first_marker_has_none() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("aaa")).expect("dirs");
        fs::create_dir_all(dir.path().join("zzz")).expect("dirs");
        fs::write(dir.path().join("aaa/.hatchdoor-layer"), "name: sources\n").expect("marker");
        fs::write(
            dir.path().join("zzz/.hatchdoor-layer"),
            "name: sources\ndescription: This is the archive.\n",
        )
        .expect("marker");

        let map = LayerMap::collect(dir.path()).expect("collect");

        // The lexicographically-first marker (aaa/) has no description, but the
        // second one (zzz/) does, so backfill applies and we get the zzz description
        assert_eq!(
            map.description("sources"),
            Some("This is the archive."),
            "description should backfill from later marker"
        );
    }
}
