//! Noise exclusion: paths that are not content at all. Gitignore syntax so
//! there is no bespoke glob dialect to document or get subtly wrong.

use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use super::layers::MARKER_FILE_NAME;

/// Applied before any user pattern, so a user `!` negation can reinstate one.
pub const DEFAULT_EXCLUDE_PATTERNS: [&str; 6] = [
    ".obsidian/",
    ".trash/",
    ".hatchdoor-trash/",
    ".DS_Store",
    "*.tmp",
    "*.sync-conflict-*",
];

pub struct ExcludeMatcher {
    inner: Gitignore,
    user_patterns: Vec<String>,
}

impl ExcludeMatcher {
    pub fn new(user_patterns: &[String]) -> Result<Self, String> {
        // The root is only used to anchor leading-`/` patterns; matching is
        // performed against vault-relative paths.
        let mut builder = GitignoreBuilder::new("");
        for pattern in DEFAULT_EXCLUDE_PATTERNS {
            builder
                .add_line(None, pattern)
                .map_err(|e| format!("invalid built-in exclude '{pattern}': {e}"))?;
        }
        for pattern in user_patterns {
            builder
                .add_line(None, pattern)
                .map_err(|e| format!("invalid HATCHDOOR_EXCLUDE pattern '{pattern}': {e}"))?;
        }
        let inner = builder
            .build()
            .map_err(|e| format!("could not build exclude matcher: {e}"))?;

        Ok(Self {
            inner,
            user_patterns: user_patterns.to_vec(),
        })
    }

    /// `relative` is vault-relative. The marker file is never excluded: a broad
    /// user pattern must not be able to disable the layer model.
    ///
    /// Uses `matched_path_or_any_parents` rather than `matched`: `matched`
    /// tests only the path's own final component, so a directory pattern like
    /// `.obsidian/` would match the directory and then report `.obsidian/
    /// workspace.json` as *not* excluded. Inside a `filter_entry` walk the
    /// pruned directory hides its children anyway, but the seeder, the
    /// diagnostic surface and any future per-path caller ask about a single
    /// path with no walk context, and they must get the right answer.
    pub fn is_excluded(&self, relative: &Path, is_dir: bool) -> bool {
        if relative.file_name().and_then(|n| n.to_str()) == Some(MARKER_FILE_NAME) {
            return false;
        }
        self.inner
            .matched_path_or_any_parents(relative, is_dir)
            .is_ignore()
    }

    /// Every pattern handed to the builder, with where it came from, for the
    /// diagnostic surface and the startup log.
    ///
    /// This reports *configured* input, not verified matcher state: a pattern
    /// appearing here means `GitignoreBuilder::add_line` accepted it, not that
    /// it was confirmed to affect any real match in `self.inner`. `ignore`
    /// does not expose per-pattern introspection (there is no "did glob N ever
    /// fire" API), so the two cannot be told apart from this list alone. A
    /// pattern can be lenient-parsed in a surprising way — e.g. an unclosed
    /// character class like `"a["` builds without error and is treated as a
    /// literal filename match rather than rejected, see
    /// `unclosed_character_class_matches_literally` below — yet it would
    /// still show up here identically to a pattern behaving exactly as
    /// written. If a pattern's effect is in question, verify it with
    /// `is_excluded` against a concrete path instead of trusting this list.
    pub fn configured_patterns(&self) -> Vec<(String, &'static str)> {
        DEFAULT_EXCLUDE_PATTERNS
            .iter()
            .map(|p| ((*p).to_string(), "built-in"))
            .chain(
                self.user_patterns
                    .iter()
                    .map(|p| (p.clone(), "HATCHDOOR_EXCLUDE")),
            )
            .collect()
    }
}

/// Manual rather than derived: `Gitignore` is opaque, and the configured
/// patterns are the only part worth printing.
impl std::fmt::Debug for ExcludeMatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExcludeMatcher")
            .field("user_patterns", &self.user_patterns)
            .finish_non_exhaustive()
    }
}

impl Default for ExcludeMatcher {
    fn default() -> Self {
        Self::new(&[]).expect("built-in exclude patterns are valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn matcher(patterns: &[&str]) -> ExcludeMatcher {
        let owned: Vec<String> = patterns.iter().map(|p| p.to_string()).collect();
        ExcludeMatcher::new(&owned).expect("valid patterns")
    }

    #[test]
    fn defaults_exclude_tooling_noise() {
        let matcher = matcher(&[]);
        assert!(matcher.is_excluded(Path::new(".obsidian"), true));
        assert!(matcher.is_excluded(Path::new(".obsidian/workspace.json"), false));
        assert!(matcher.is_excluded(Path::new(".hatchdoor-trash"), true));
        assert!(matcher.is_excluded(Path::new("notes/.DS_Store"), false));
        assert!(matcher.is_excluded(Path::new("notes/draft.tmp"), false));
        assert!(matcher.is_excluded(Path::new("notes/A.sync-conflict-2026.md"), false));
        assert!(!matcher.is_excluded(Path::new("notes/Real Note.md"), false));
    }

    #[test]
    fn user_patterns_append_and_negation_reinstates_a_default() {
        let matcher = matcher(&["build/", "!.DS_Store"]);
        assert!(matcher.is_excluded(Path::new("build"), true));
        // A later `!` pattern wins under gitignore semantics, which is how a
        // deployment drops one built-in without discarding the whole set.
        assert!(!matcher.is_excluded(Path::new("notes/.DS_Store"), false));
    }

    #[test]
    fn marker_file_is_immune_to_every_pattern() {
        // A broad `.*` rule must not be able to silently disable the layer model.
        let matcher = matcher(&[".*"]);
        assert!(!matcher.is_excluded(Path::new("sources/.hatchdoor-layer"), false));
        assert!(matcher.is_excluded(Path::new("sources/.other-dotfile"), false));
    }

    #[test]
    fn configured_patterns_report_provenance() {
        let matcher = matcher(&["build/"]);
        let patterns = matcher.configured_patterns();
        assert!(patterns.contains(&(".DS_Store".to_string(), "built-in")));
        assert!(patterns.contains(&("build/".to_string(), "HATCHDOOR_EXCLUDE")));
    }

    #[test]
    fn unclosed_character_class_matches_literally() {
        // `"a["` is a malformed gitignore glob (an unclosed character class),
        // but `ignore` is lenient: `add_line`/`build` both succeed, and the
        // pattern is incorporated as a *literal* match on the exact string
        // "a[" rather than being rejected or silently dropped. It does NOT
        // behave as a class matching e.g. "a" or "ab". Pin this exact
        // behaviour through `is_excluded` (the real matcher), not through
        // `configured_patterns` (which only echoes configured input and would
        // report this pattern as present regardless of whether `ignore`
        // incorporated it) — see the doc comment on `configured_patterns`.
        // If a future `ignore` upgrade starts rejecting this pattern at
        // construction, `matcher()` here will panic and this test will fail
        // loudly, which is the desired outcome.
        let matcher = matcher(&["a["]);
        assert!(matcher.is_excluded(Path::new("a["), false));
        assert!(!matcher.is_excluded(Path::new("a"), false));
        assert!(!matcher.is_excluded(Path::new("ab"), false));
        assert!(!matcher.is_excluded(Path::new("a[b"), false));
    }
}
