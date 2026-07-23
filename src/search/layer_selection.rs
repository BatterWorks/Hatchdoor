//! Layer selection for read surfaces.
//!
//! A `LayerSelection` says which vault layers a read surface should return. It
//! is a **selector, not an additive flag**: omitting it means the default
//! surface only, naming a set means exactly those layers, and `all` means
//! everything. See the design doc's "MCP surface" section for the semantics
//! these three states encode.

use std::collections::BTreeSet;

/// Reserved selector tokens. Neither is ever a layer name (both are rejected as
/// marker names at startup), so they never collide with a real layer.
pub const TOKEN_DEFAULT: &str = "default";
pub const TOKEN_ALL: &str = "all";

/// Which layers a read surface includes.
///
/// - [`LayerSelection::Set`] with `include_default` and a (possibly empty) set
///   of named layers covers "default surface only", "named layers only" and
///   "default plus named".
/// - [`LayerSelection::All`] covers everything with no `notes.layer` predicate,
///   so default search keeps its unfiltered fast path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerSelection {
    /// Every layer, default surface included. No predicate on `notes.layer`.
    All,
    /// The default surface (`layer IS NULL`) and/or a set of named demoted
    /// layers. The empty-set, `include_default = true` case is the default
    /// surface only.
    Set {
        include_default: bool,
        layers: BTreeSet<String>,
    },
}

impl Default for LayerSelection {
    /// An omitted/empty selection is the default surface only.
    fn default() -> Self {
        LayerSelection::Set {
            include_default: true,
            layers: BTreeSet::new(),
        }
    }
}

impl LayerSelection {
    /// The default surface only (`layer IS NULL`).
    pub fn default_surface() -> Self {
        LayerSelection::default()
    }

    /// Everything, with no layer filtering.
    pub fn all() -> Self {
        LayerSelection::All
    }

    /// Parse selector tokens against the vault's known layer names.
    ///
    /// - An empty token list is the default surface only.
    /// - `all` anywhere selects everything.
    /// - `default` adds the default surface.
    /// - Any other token must match a known layer name.
    ///
    /// An unknown layer name is **not** a hard error: it degrades to the
    /// default surface with a warning, so a stale MCP client holding a removed
    /// layer name keeps working instead of hard-failing. Returned warnings are
    /// for the caller to log/surface.
    pub fn parse(tokens: &[String], known_layers: &[String]) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();

        if tokens.is_empty() {
            return (Self::default(), warnings);
        }

        if tokens
            .iter()
            .any(|token| token.trim().eq_ignore_ascii_case(TOKEN_ALL))
        {
            return (LayerSelection::All, warnings);
        }

        let mut include_default = false;
        let mut layers = BTreeSet::new();
        for token in tokens {
            let trimmed = token.trim();
            if trimmed.eq_ignore_ascii_case(TOKEN_DEFAULT) {
                include_default = true;
            } else if known_layers.iter().any(|known| known == trimmed) {
                layers.insert(trimmed.to_string());
            } else {
                warnings.push(format!(
                    "unknown layer '{trimmed}' ignored; falling back to the default surface"
                ));
                include_default = true;
            }
        }

        // Only reachable if every token was blank; treat as the default surface.
        if !include_default && layers.is_empty() {
            include_default = true;
        }

        (
            LayerSelection::Set {
                include_default,
                layers,
            },
            warnings,
        )
    }

    /// A SQL boolean expression over `column` (e.g. `"layer"`, `"n.layer"`,
    /// `"target.layer"`) that is true for exactly the rows this selection
    /// includes. Splice it into a `WHERE`/`AND` clause.
    ///
    /// Layer names are inlined as quoted string literals. This is
    /// injection-safe: a layer name is validated to `[alnum][alnum-]{0,31}` by
    /// `normalize_layer_name` before it can appear in a `LayerMap`, and
    /// [`LayerSelection::parse`] only admits names already present there — an
    /// unknown token degrades rather than reaching SQL. Single quotes are
    /// doubled defensively regardless.
    pub fn sql_filter(&self, column: &str) -> String {
        match self {
            LayerSelection::All => "1".to_string(),
            LayerSelection::Set {
                include_default,
                layers,
            } => {
                let mut clauses: Vec<String> = Vec::new();
                if *include_default {
                    clauses.push(format!("{column} IS NULL"));
                }
                if !layers.is_empty() {
                    let list = layers
                        .iter()
                        .map(|name| format!("'{}'", name.replace('\'', "''")))
                        .collect::<Vec<_>>()
                        .join(", ");
                    clauses.push(format!("{column} IN ({list})"));
                }
                match clauses.len() {
                    0 => "0".to_string(),
                    1 => clauses.pop().expect("one clause"),
                    _ => format!("({})", clauses.join(" OR ")),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<String> {
        vec!["sources".to_string(), "archive-notes".to_string()]
    }

    #[test]
    fn omitted_selection_is_the_default_surface() {
        let (selection, warnings) = LayerSelection::parse(&[], &known());
        assert_eq!(selection, LayerSelection::default_surface());
        assert!(warnings.is_empty());
        assert_eq!(selection.sql_filter("layer"), "layer IS NULL");
    }

    #[test]
    fn named_layer_selects_that_layer_only() {
        let (selection, warnings) = LayerSelection::parse(&["sources".to_string()], &known());
        assert!(warnings.is_empty());
        assert_eq!(
            selection,
            LayerSelection::Set {
                include_default: false,
                layers: ["sources".to_string()].into_iter().collect(),
            }
        );
        assert_eq!(selection.sql_filter("layer"), "layer IN ('sources')");
    }

    #[test]
    fn default_plus_named_includes_both() {
        let (selection, warnings) =
            LayerSelection::parse(&["default".to_string(), "sources".to_string()], &known());
        assert!(warnings.is_empty());
        assert_eq!(
            selection.sql_filter("n.layer"),
            "(n.layer IS NULL OR n.layer IN ('sources'))"
        );
    }

    #[test]
    fn all_token_selects_everything() {
        let (selection, warnings) = LayerSelection::parse(&["all".to_string()], &known());
        assert!(warnings.is_empty());
        assert_eq!(selection, LayerSelection::All);
        assert_eq!(selection.sql_filter("layer"), "1");
    }

    #[test]
    fn all_wins_over_other_tokens() {
        let (selection, _) =
            LayerSelection::parse(&["sources".to_string(), "all".to_string()], &known());
        assert_eq!(selection, LayerSelection::All);
    }

    #[test]
    fn unknown_layer_degrades_to_default_with_a_warning() {
        let (selection, warnings) = LayerSelection::parse(&["ghost".to_string()], &known());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("ghost"));
        // Degrades to the default surface, not a hard error.
        assert_eq!(selection, LayerSelection::default_surface());
        assert_eq!(selection.sql_filter("layer"), "layer IS NULL");
    }

    #[test]
    fn unknown_layer_alongside_a_known_one_keeps_the_known_and_adds_default() {
        let (selection, warnings) =
            LayerSelection::parse(&["sources".to_string(), "ghost".to_string()], &known());
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            selection,
            LayerSelection::Set {
                include_default: true,
                layers: ["sources".to_string()].into_iter().collect(),
            }
        );
        assert_eq!(
            selection.sql_filter("layer"),
            "(layer IS NULL OR layer IN ('sources'))"
        );
    }

    #[test]
    fn two_named_layers_sort_deterministically_in_sql() {
        let (selection, _) = LayerSelection::parse(
            &["sources".to_string(), "archive-notes".to_string()],
            &known(),
        );
        assert_eq!(
            selection.sql_filter("layer"),
            "layer IN ('archive-notes', 'sources')"
        );
    }

    #[test]
    fn reserved_tokens_are_case_insensitive() {
        let (selection, warnings) = LayerSelection::parse(&["ALL".to_string()], &known());
        assert!(warnings.is_empty());
        assert_eq!(selection, LayerSelection::All);
    }
}
