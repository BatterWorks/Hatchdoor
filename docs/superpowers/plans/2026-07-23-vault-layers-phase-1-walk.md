# Vault Layers — Phase 1 (Walk Level) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Teach the vault walk to classify every note into a named demoted layer, the default surface, or noise — with nothing downstream consuming the classification yet.

**Architecture:** Two new pure modules under `src/vault/`: `layers.rs` (marker parsing, name normalization, prefix-based resolution) and `exclude.rs` (gitignore-syntax noise matching). `VaultIndex::build` gains a sibling `build_with_config` that threads a `VaultScanConfig`; the existing `build` delegates with defaults so the ~45 existing call sites stay untouched. Slug allocation is reordered so default-surface notes win collisions. Everything in this phase is testable without touching SQLite, embeddings, MCP, or the frontend.

**Tech Stack:** Rust 1.96.0, `walkdir 2` (already a dep), `serde_yaml 0.9` (already a dep), `ignore` (new), `unicode-normalization` (new), `tempfile` (dev-dep).

**Spec:** `docs/superpowers/specs/2026-07-23-vault-layers-and-exclusions-design.md` — sections *Marker file*, *Noise*, *Addressing*.

## Global Constraints

- Toolchain is pinned to Rust 1.96.0 in `rust-toolchain.toml`. Do not bump it. Run `cargo fmt` before every commit.
- Fallible functions return `Result<T, String>`, matching the existing precedent in `AppConfig::from_env` (`src/config.rs:27`). Do not add `thiserror` or `anyhow`.
- Marker file name is exactly `.hatchdoor-layer`.
- Reserved layer names, rejected as marker names: `default`, `all`, `noise`, `none`.
- Layer names, after NFKC → trim → lowercase → spaces-to-`-`, must be alphanumeric characters plus `-`, starting with an alphanumeric, at most 32 **characters** (not bytes). Alphanumeric is Unicode-wide (`char::is_alphanumeric`), not ASCII-only: `sources-privées` and `資料` are valid. The whitelist is the security mechanism — zero-width characters, bidi overrides, control characters, punctuation and emoji are non-alphanumeric and therefore already refused.
- Descriptions are capped at 500 characters after sanitization.
- Built-in noise patterns, in this order: `.obsidian/`, `.trash/`, `.hatchdoor-trash/`, `.DS_Store`, `*.tmp`, `*.sync-conflict-*`.
- `.hatchdoor-layer` is never excluded by any pattern, built-in or user-supplied.
- Directory symlinks are not followed.
- **Nothing in this phase changes observable behaviour of the running app, except for the default noise set.** `VaultIndex::build` must keep producing identical output for a vault with no markers and no configured excludes, with one deliberate exception: every path matching the built-in noise patterns is no longer indexed, where previously only `.hatchdoor-trash` was skipped.

  This exception is wider than it looks and must be stated accurately, because three of the six default patterns can match `.md` files that were previously indexed and searchable:
  - `.trash/` holds Obsidian's locally deleted notes.
  - `*.sync-conflict-*` matches Syncthing conflict files, which are `.md`.
  - `.obsidian/` can hold plugin documentation and some template setups.

  Notes under those paths disappear from search and from wikilink resolution after upgrade. That is the intended behaviour of the feature, but it is a user-visible index change and requires a release note.

## File Structure

| File | Responsibility |
|---|---|
| `src/vault/layers.rs` (new) | Marker file parsing, layer-name normalization, `LayerMap` prefix resolution. No filesystem walking policy — just "given a root, collect markers" and "given a relative path, tell me the layer". |
| `src/vault/exclude.rs` (new) | Noise matching only. Wraps `ignore::gitignore`, owns the built-in defaults, enforces marker immunity, reports effective patterns with provenance. |
| `src/vault/types.rs` (modify) | `NoteEntry.layer` field; `VaultScanConfig` struct. |
| `src/vault/index.rs` (modify) | `build_with_config`; noise pruning; layer assignment; slug-precedence ordering. |
| `src/vault/seed.rs` (modify) | `has_markdown_notes` uses the shared exclude matcher instead of its duplicated `.hatchdoor-trash` filter. |
| `src/vault.rs` (modify) | Module declarations and re-exports. |
| `Cargo.toml` (modify) | Add `ignore`, `unicode-normalization`. |

Tests live inline as `#[cfg(test)] mod tests` in `layers.rs` and `exclude.rs` (matching `src/config.rs` and `src/mcp/config.rs`), and in `src/vault/tests.rs` for the integration-level walk behaviour (matching the existing `build_indexes_markdown_files_only` test).

---

### Task 1: Layer name normalization

**Files:**
- Create: `src/vault/layers.rs`
- Modify: `src/vault.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn normalize_layer_name(raw: &str) -> Result<String, String>`; `pub const MARKER_FILE_NAME: &str`; `pub const RESERVED_LAYER_NAMES: [&str; 4]`.

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, under `[dependencies]`, add alongside the existing entries:

```toml
unicode-normalization = "0.1"
```

- [ ] **Step 2: Write the failing test**

Create `src/vault/layers.rs` containing only:

```rust
//! Layer markers: parsing `.hatchdoor-layer` files and resolving which layer a
//! vault path belongs to. Pure logic — the walk policy lives in `index.rs`.

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
```

Add to `src/vault.rs`, keeping the `mod` list alphabetical:

```rust
mod exclude;
mod index;
mod layers;
mod links;
```

(`exclude` is declared now so Task 3 does not have to revisit this file; create an empty `src/vault/exclude.rs` with just `//! Noise exclusion patterns.` for the moment.)

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --lib vault::layers`
Expected: FAIL — `cannot find function 'normalize_layer_name' in this scope`.

- [ ] **Step 4: Write minimal implementation**

Above the `mod tests` block in `src/vault/layers.rs`:

```rust
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
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --lib vault::layers`
Expected: PASS, 3 tests.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add Cargo.toml Cargo.lock src/vault.rs src/vault/layers.rs src/vault/exclude.rs
git commit -m "feat(vault): normalize and validate layer names"
```

---

### Task 2: Marker file parsing

**Files:**
- Modify: `src/vault/layers.rs`

**Interfaces:**
- Consumes: `normalize_layer_name` from Task 1.
- Produces: `pub enum LayerDecl { Default, Named { name: String, description: Option<String> } }`; `pub fn parse_marker(contents: &str) -> Result<LayerDecl, String>`.

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block in `src/vault/layers.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib vault::layers`
Expected: FAIL — `cannot find function 'parse_marker'`, `cannot find type 'LayerDecl'`.

- [ ] **Step 3: Write minimal implementation**

Add to `src/vault/layers.rs`, above `mod tests`:

```rust
use serde::Deserialize;

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
    let raw: RawMarker = serde_yaml::from_str(contents)
        .map_err(|e| format!("malformed {MARKER_FILE_NAME}: {e}"))?;

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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib vault::layers`
Expected: PASS, 8 tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/vault/layers.rs
git commit -m "feat(vault): parse .hatchdoor-layer marker files"
```

---

### Task 3: Noise exclusion matcher

**Files:**
- Modify: `src/vault/exclude.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `MARKER_FILE_NAME` from Task 1.
- Produces: `pub struct ExcludeMatcher`; `ExcludeMatcher::new(user_patterns: &[String]) -> Result<Self, String>`; `ExcludeMatcher::is_excluded(&self, relative: &Path, is_dir: bool) -> bool`; `ExcludeMatcher::effective_patterns(&self) -> Vec<(String, &'static str)>`; `impl Default for ExcludeMatcher`; `pub const DEFAULT_EXCLUDE_PATTERNS: [&str; 6]`.

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, under `[dependencies]`:

```toml
ignore = "0.4"
```

- [ ] **Step 2: Write the failing test**

Replace the contents of `src/vault/exclude.rs` with:

```rust
//! Noise exclusion: paths that are not content at all. Gitignore syntax so
//! there is no bespoke glob dialect to document or get subtly wrong.

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
    fn effective_patterns_report_provenance() {
        let matcher = matcher(&["build/"]);
        let patterns = matcher.effective_patterns();
        assert!(patterns.contains(&(".DS_Store".to_string(), "built-in")));
        assert!(patterns.contains(&("build/".to_string(), "HATCHDOOR_EXCLUDE")));
    }

    #[test]
    fn invalid_pattern_is_rejected() {
        let bad = vec!["[".to_string()];
        assert!(ExcludeMatcher::new(&bad).is_err());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --lib vault::exclude`
Expected: FAIL — `cannot find type 'ExcludeMatcher' in this scope`.

- [ ] **Step 4: Write minimal implementation**

Add above `mod tests` in `src/vault/exclude.rs`:

```rust
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

    /// Every active pattern with where it came from, for the diagnostic surface
    /// and the startup log.
    pub fn effective_patterns(&self) -> Vec<(String, &'static str)> {
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

impl Default for ExcludeMatcher {
    fn default() -> Self {
        Self::new(&[]).expect("built-in exclude patterns are valid")
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --lib vault::exclude`
Expected: PASS, 5 tests.

If `invalid_pattern_is_rejected` fails, `ignore` accepts `[` rather than
rejecting it. Do not delete the test — rewrite it to assert the behaviour you
actually observe, so the crate's contract stays pinned:

```rust
#[test]
fn unparseable_pattern_is_surfaced_not_silently_dropped() {
    // `ignore` is lenient about some malformed patterns. Whatever it does,
    // pin it: a pattern must either be rejected at construction or take
    // effect — it must never be silently ignored.
    let matcher = matcher(&["a["]);
    let patterns = matcher.effective_patterns();
    assert!(patterns.contains(&("a[".to_string(), "HATCHDOOR_EXCLUDE")));
}
```

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add Cargo.toml Cargo.lock src/vault/exclude.rs
git commit -m "feat(vault): gitignore-syntax noise exclusion matcher"
```

---

### Task 4: Collect markers and resolve paths to layers

**Files:**
- Modify: `src/vault/layers.rs`

**Interfaces:**
- Consumes: `parse_marker`, `LayerDecl`, `MARKER_FILE_NAME` (Tasks 1–2).
- Produces: `pub struct LayerMap`; `LayerMap::collect(root: &Path) -> Result<Self, String>`; `LayerMap::layer_for(&self, relative_path: &str) -> Option<&str>`; `LayerMap::layer_names(&self) -> Vec<String>`; `LayerMap::description(&self, name: &str) -> Option<&str>`; `LayerMap::marker_paths(&self) -> Vec<String>`; `impl Default for LayerMap`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/vault/layers.rs` (add `use std::fs;` and `use tempfile::tempdir;` to the test module's imports):

```rust
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
    assert!(err.contains("sources/.hatchdoor-layer"), "unexpected: {err}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib vault::layers`
Expected: FAIL — `cannot find type 'LayerMap' in this scope`.

- [ ] **Step 3: Write minimal implementation**

Add above `mod tests` in `src/vault/layers.rs`:

```rust
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use walkdir::WalkDir;

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
            if !entry.file_type().is_file()
                || entry.file_name().to_str() != Some(MARKER_FILE_NAME)
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
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib vault::layers`
Expected: PASS, 14 tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/vault/layers.rs
git commit -m "feat(vault): collect layer markers and resolve paths to layers"
```

---

### Task 5: Frontmatter file-level layer override

**Files:**
- Modify: `src/vault/layers.rs`

**Interfaces:**
- Consumes: `normalize_layer_name`, `LayerDecl` (Tasks 1–2).
- Produces: `pub fn layer_from_frontmatter(properties: &serde_json::Value) -> Result<Option<LayerDecl>, String>`.

Markers are folder-scoped, which cannot express individual files that are content but not a browsing surface (`log.md`, `README.md`). Moving such a file into a demoted folder would change its path and break its wikilinks.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/vault/layers.rs`:

```rust
#[test]
fn frontmatter_declares_a_file_level_layer() {
    let properties = serde_json::json!({ "hatchdoor": { "layer": "sources" } });
    assert_eq!(
        layer_from_frontmatter(&properties).expect("valid"),
        Some(LayerDecl::Named {
            name: "sources".to_string(),
            description: None
        })
    );
}

#[test]
fn frontmatter_default_reincludes_a_single_file() {
    let properties = serde_json::json!({ "hatchdoor": { "layer": "default" } });
    assert_eq!(
        layer_from_frontmatter(&properties).expect("valid"),
        Some(LayerDecl::Default)
    );
}

#[test]
fn frontmatter_without_hatchdoor_key_declares_nothing() {
    let properties = serde_json::json!({ "tags": ["a"] });
    assert_eq!(layer_from_frontmatter(&properties).expect("valid"), None);
    assert_eq!(
        layer_from_frontmatter(&serde_json::Value::Null).expect("valid"),
        None
    );
}

#[test]
fn frontmatter_rejects_reserved_layer_name() {
    let properties = serde_json::json!({ "hatchdoor": { "layer": "noise" } });
    assert!(layer_from_frontmatter(&properties).is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib vault::layers`
Expected: FAIL — `cannot find function 'layer_from_frontmatter'`.

- [ ] **Step 3: Write minimal implementation**

Add above `mod tests` in `src/vault/layers.rs`:

```rust
/// A note may override its inherited folder marker with frontmatter:
///
/// ```yaml
/// hatchdoor:
///   layer: sources
/// ```
///
/// Frontmatter is already parsed, is Obsidian-native, and travels with the file
/// when it moves.
pub fn layer_from_frontmatter(
    properties: &serde_json::Value,
) -> Result<Option<LayerDecl>, String> {
    let Some(raw) = properties
        .get("hatchdoor")
        .and_then(|section| section.get("layer"))
        .and_then(|value| value.as_str())
    else {
        return Ok(None);
    };

    if raw.nfkc().collect::<String>().trim().to_lowercase() == "default" {
        return Ok(Some(LayerDecl::Default));
    }

    Ok(Some(LayerDecl::Named {
        name: normalize_layer_name(raw)?,
        description: None,
    }))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib vault::layers`
Expected: PASS, 18 tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/vault/layers.rs
git commit -m "feat(vault): file-level layer override via frontmatter"
```

---

### Task 6: Thread layers and noise through the walk

**Files:**
- Modify: `src/vault/types.rs`
- Modify: `src/vault/index.rs:19-100`
- Modify: `src/vault.rs`
- Test: `src/vault/tests.rs`

**Interfaces:**
- Consumes: `LayerMap`, `ExcludeMatcher` (Tasks 3–4).
- Produces: `pub struct VaultScanConfig { pub exclude: ExcludeMatcher }`; `VaultIndex::build_with_config(root, &VaultScanConfig) -> io::Result<Self>`; `VaultIndex.layers: LayerMap`; `NoteEntry.layer: Option<String>`. `VaultIndex::build(root)` keeps its existing signature and delegates with defaults, so the ~45 existing call sites do not change.

- [ ] **Step 1: Write the failing test**

Add to `src/vault/tests.rs`:

```rust
#[test]
fn build_assigns_layers_and_skips_noise() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("sources")).expect("dirs");
    fs::create_dir_all(dir.path().join("wiki")).expect("dirs");
    fs::create_dir_all(dir.path().join(".obsidian")).expect("dirs");
    fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");
    fs::write(dir.path().join("sources/Clipping.md"), "# Clipping").expect("note");
    fs::write(dir.path().join("wiki/Topic.md"), "# Topic").expect("note");
    fs::write(dir.path().join(".obsidian/Plugin Notes.md"), "# Noise").expect("note");
    fs::write(dir.path().join("wiki/Scratch.tmp"), "ignored").expect("tmp");

    let index = VaultIndex::build(dir.path()).expect("index");

    let layer_of = |title: &str| {
        index
            .by_slug
            .values()
            .find(|entry| entry.title == title)
            .map(|entry| entry.layer.clone())
    };

    assert_eq!(layer_of("Clipping"), Some(Some("sources".to_string())));
    assert_eq!(layer_of("Topic"), Some(None));
    assert_eq!(layer_of("Plugin Notes"), None, "noise must not be indexed");
    assert_eq!(index.layers.layer_names(), vec!["sources".to_string()]);
}

#[test]
fn build_gives_the_unsuffixed_slug_to_the_default_surface() {
    // A compiled page named after the source it compiles is the normal case in
    // a layered vault, and `sources/` sorts before `wiki/`. Without precedence
    // every [[Melatonin]] would resolve to the clipping.
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("sources")).expect("dirs");
    fs::create_dir_all(dir.path().join("wiki")).expect("dirs");
    fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");
    fs::write(dir.path().join("sources/Melatonin.md"), "# Source").expect("note");
    fs::write(dir.path().join("wiki/Melatonin.md"), "# Compiled").expect("note");

    let index = VaultIndex::build(dir.path()).expect("index");

    let compiled = index.find_by_slug("melatonin").expect("slug melatonin");
    assert_eq!(compiled.relative_path, "wiki/Melatonin");
    assert_eq!(compiled.layer, None);

    let source = index.find_by_slug("melatonin-2").expect("slug melatonin-2");
    assert_eq!(source.relative_path, "sources/Melatonin");
    assert_eq!(source.layer.as_deref(), Some("sources"));

    assert_eq!(
        index.resolve_wikilink("Melatonin").map(|e| e.slug.as_str()),
        Some("melatonin")
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib vault::tests`
Expected: FAIL — `no field 'layer' on type 'NoteEntry'`, `no field 'layers' on type 'VaultIndex'`.

- [ ] **Step 3: Add the types**

In `src/vault/types.rs`, add `layer` to `NoteEntry` (line 7) and `layers` to `VaultIndex` (line 40), and add the config struct:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteEntry {
    pub title: String,
    pub slug: String,
    pub path: PathBuf,
    pub relative_path: String,
    /// `None` is the default surface.
    pub layer: Option<String>,
}
```

```rust
#[derive(Debug, Clone)]
pub struct VaultIndex {
    pub by_slug: HashMap<String, NoteEntry>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub by_title: HashMap<String, String>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub by_path_title: HashMap<String, String>,
    pub ordered_slugs: Vec<String>,
    pub outgoing_by_slug: HashMap<String, Vec<String>>,
    pub backlinks_by_slug: HashMap<String, Vec<String>>,
    pub layers: super::layers::LayerMap,
}
```

Add at the end of `src/vault/types.rs`:

```rust
/// Deployment-side scan configuration. Layer classification comes from the
/// vault itself and is not represented here.
#[derive(Debug, Default)]
pub struct VaultScanConfig {
    pub exclude: super::exclude::ExcludeMatcher,
}
```

`VaultScanConfig` derives `Debug`, so `ExcludeMatcher` needs a `Debug` impl. Do
not derive it — `ignore::gitignore::Gitignore` may not implement `Debug`. Add a
manual impl to `src/vault/exclude.rs` that reports only the configured patterns:

```rust
impl std::fmt::Debug for ExcludeMatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExcludeMatcher")
            .field("user_patterns", &self.user_patterns)
            .finish_non_exhaustive()
    }
}
```

- [ ] **Step 4: Rewrite the walk**

Replace `src/vault/index.rs:19-45` (the start of `build` through the `markdown_paths.sort();` line) with:

```rust
impl VaultIndex {
    /// Scans with default deployment configuration. Retained so existing
    /// callers and tests are unaffected.
    pub fn build(root: impl AsRef<Path>) -> io::Result<Self> {
        Self::build_with_config(root, &VaultScanConfig::default())
    }

    pub fn build_with_config(
        root: impl AsRef<Path>,
        config: &VaultScanConfig,
    ) -> io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        let mut by_slug = HashMap::new();
        let mut by_title = HashMap::new();
        let mut by_path_title = HashMap::new();
        let mut ordered_slugs = Vec::new();
        let mut markdown_paths = Vec::new();

        // Markers are collected before pruning: a marker inside a directory a
        // noise pattern would prune is still read, so per-deployment noise
        // cannot silently delete a layer.
        let layers = LayerMap::collect(&root).map_err(io::Error::other)?;

        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                entry.depth() == 0
                    || match entry.path().strip_prefix(&root) {
                        Ok(relative) => !config
                            .exclude
                            .is_excluded(relative, entry.file_type().is_dir()),
                        Err(_) => true,
                    }
            })
        {
            let entry = entry.map_err(io::Error::other)?;
            let path = entry.path();

            if !entry.file_type().is_file()
                || path.extension().and_then(|ext| ext.to_str()) != Some("md")
            {
                continue;
            }
            markdown_paths.push(path.to_path_buf());
        }

        markdown_paths.sort();
        // Default-surface notes claim their slugs first, so a compiled page
        // beats the source it was compiled from on a title collision.
        markdown_paths.sort_by_key(|path| {
            relative_note_path_without_ext(&root, path)
                .map(|relative| layers.layer_for(&relative).is_some())
                .unwrap_or(false)
        });
```

`sort_by_key` is stable, so within each group the original path order is preserved.

In the same function, set `layer` when constructing the `NoteEntry` (currently `src/vault/index.rs:60`):

```rust
            let note = NoteEntry {
                title: stem.clone(),
                slug: slug.clone(),
                path: path.to_path_buf(),
                relative_path: relative_without_ext.clone(),
                layer: layers.layer_for(&relative_without_ext).map(str::to_string),
            };
```

And add `layers` to the returned struct (currently `src/vault/index.rs:92`):

```rust
        Ok(Self {
            by_slug,
            by_title,
            by_path_title,
            ordered_slugs,
            outgoing_by_slug,
            backlinks_by_slug,
            layers,
        })
```

Update the imports at the top of `src/vault/index.rs`:

```rust
use super::exclude::ExcludeMatcher;
use super::layers::LayerMap;
use super::types::{
    ExplorerFolder, ExplorerNote, Note, NoteEntry, NoteLink, NoteLinks, SearchHit, VaultIndex,
    VaultScanConfig,
};
```

(`ExcludeMatcher` is imported for the `VaultScanConfig` field type; drop the import if the compiler reports it unused.)

Export the new items from `src/vault.rs`:

```rust
pub use exclude::{DEFAULT_EXCLUDE_PATTERNS, ExcludeMatcher};
pub use layers::{LayerDecl, LayerMap, MARKER_FILE_NAME, layer_from_frontmatter};
pub use types::{
    ExplorerFolder, ExplorerNote, ModifiedNote, Note, NoteEntry, NoteLink, NoteLinks, NoteMetadata,
    NoteSummary, SearchHit, VaultIndex, VaultScanConfig,
};
```

- [ ] **Step 5: Fix every `NoteEntry` literal**

Run: `cargo test --lib 2>&1 | grep -c "missing field \`layer\`"`

Every `NoteEntry { .. }` literal in the codebase needs `layer: None`. Find them:

Run: `grep -rn "NoteEntry {" src/`

Add `layer: None` to each. Do not change any other field.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --lib vault`
Expected: PASS, including the two new tests and all pre-existing vault tests unchanged.

Run: `cargo test`
Expected: PASS. If `build_indexes_markdown_files_only` fails, the `.hatchdoor-trash` default pattern is not matching — check that `.hatchdoor-trash/` is in `DEFAULT_EXCLUDE_PATTERNS` and that `filter_entry` is receiving a vault-relative path.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/vault.rs src/vault/types.rs src/vault/index.rs src/vault/exclude.rs src/vault/tests.rs
# Plus every file Step 5 touched to add `layer: None` — list them explicitly
# from `git status`; do not `git add src/` wholesale.
git commit -m "feat(vault): classify notes by layer and prune noise during the walk"
```

---

### Task 7: Share the exclude matcher with the seeder

**Files:**
- Modify: `src/vault/seed.rs:69-83`
- Test: `src/vault/tests.rs`

**Interfaces:**
- Consumes: `ExcludeMatcher` (Task 3).
- Produces: no new public interface.

`has_markdown_notes` duplicates the hardcoded `.hatchdoor-trash` filter and would count noise files, suppressing starter-vault seeding for a vault whose only markdown lives in `.obsidian/`.

- [ ] **Step 1: Write the failing test**

Add to `src/vault/tests.rs`:

```rust
#[test]
fn seeding_is_not_suppressed_by_noise_only_markdown() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join(".obsidian")).expect("dirs");
    fs::write(dir.path().join(".obsidian/Plugin Notes.md"), "# Noise").expect("note");

    // The vault has no real content, so the starter vault must still be written.
    let seeded = crate::vault::seed_empty_vault(dir.path()).expect("seed");
    assert!(seeded, "noise-only markdown must not count as content");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib vault::tests::seeding_is_not_suppressed_by_noise_only_markdown`
Expected: FAIL — assertion failed: `noise-only markdown must not count as content`.

- [ ] **Step 3: Write minimal implementation**

In `src/vault/seed.rs`, replace `has_markdown_notes`:

```rust
fn has_markdown_notes(root: &Path) -> io::Result<bool> {
    let exclude = ExcludeMatcher::default();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || match entry.path().strip_prefix(root) {
                    Ok(relative) => !exclude.is_excluded(relative, entry.file_type().is_dir()),
                    Err(_) => true,
                }
        })
    {
        let entry = entry.map_err(io::Error::other)?;
        let path = entry.path();
        if entry.file_type().is_file()
            && path.extension().and_then(|ext| ext.to_str()) == Some("md")
        {
            return Ok(true);
        }
    }

    Ok(false)
}
```

Add to the imports at the top of `src/vault/seed.rs`:

```rust
use super::exclude::ExcludeMatcher;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib vault`
Expected: PASS.

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: PASS with no warnings.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/vault/seed.rs src/vault/tests.rs
git commit -m "fix(vault): seeder ignores noise when checking for existing notes"
```

---

## Phase exit criteria

- `cargo test` and `cargo clippy --all-targets -- -D warnings` pass.
- A vault with no markers and no configured excludes produces the same index as before this phase.
- `NoteEntry.layer` is populated and `VaultIndex.layers` is available, with **no** consumer reading either yet — the app's observable behaviour is unchanged.

## Deferred to later phases

Written down so nothing here is mistaken for an omission. Each becomes its own plan.

- **Phase 2 — Cache:** `notes.layer` column, layer on link/edge rows, `VaultStats`/`GraphResponse`, marker-set hash forcing a full note-row refresh, the refuse-silent-promotion guard, `SCHEMA_VERSION` bump and its full-re-embed migration note.
- **Phase 3 — Vectors and search:** per-layer vec0 tables, `layers` selector semantics, `path_prefix` precedence error, `HATCHDOOR_EMBED_LAYERS` in the embedding cache key.
- **Phase 4 — MCP:** per-vault enum generation, `tools/list_changed`, `get_note` path argument, `recently_modified` exposure, layer on responses and write outcomes.
- **Phase 5 — Frontend:** `layer` on the seven note-identity types, tree/search/graph filtering, reveal toggle, badges, autocomplete keeping demoted candidates, resolve-batch layer signal.
- **Phase 6 — Config, demo_mode and diagnostics:** `HATCHDOOR_EXCLUDE` wiring into `AppConfig` and `VaultScanConfig`, effective-pattern startup log, watcher noise filtering and marker-triggered full reindex, runtime malformed-marker last-good retention, server-side layer-parameter rejection under `demo_mode`, the three-output diagnostic surface.

  **Phase 6 also owns the startup failure path, which phase 1 leaves unrecoverable.** A malformed marker fails the index build; `src/server.rs:413` then marks startup failed and skips spawning both the vault watcher and git sync, so the server serves the stale cache forever and fixing the marker on disk does nothing until a restart. Phase 6 must spawn the watcher in the failure arm and clear the failed state on a successful recovery. Reconcile at the same time with the codebase's warn-and-degrade convention for malformed vault YAML (`src/cache/populate.rs:896`).

- **Unassigned, needs a phase:** `layer_from_frontmatter` is implemented and exported but has no consumer and appears in no phase. The spec's requirement that write tools refuse to create `.hatchdoor-layer` files also has no phase. Both need owners before they rot.

Note that until phase 6 lands, `HATCHDOOR_EXCLUDE` is not yet read from the environment: this phase builds the mechanism and wires only the built-in defaults.
