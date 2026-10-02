//! A vault's link style (ADR-33): which form new note links and attachment
//! embeds take, wikilinks or Markdown links, and for Markdown links which path
//! form. It is read from the vault each time it is asked for and never saved;
//! only per-note link counts are kept in memory, to avoid re-reading notes.
//!
//! Obsidian's own record of the choice, `.obsidian/app.json`, wins when the
//! vault has one. Without it the vault's existing links vote. Hatchdoor only
//! ever reads that file.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const OBSIDIAN_APP_CONFIG: &str = ".obsidian/app.json";

/// The syntax new links are written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkStyle {
    /// `[[Title]]` and `![[path]]`.
    Wikilink,
    /// `[Title](path.md)` and `![](path)`.
    Markdown,
}

/// How a Markdown link writes its path. Obsidian's `newLinkFormat` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkPathForm {
    /// Relative to the linking note's folder.
    Relative,
    /// From the vault root, with a leading `/`.
    Absolute,
    /// The bare file name when it is unique in the vault, otherwise the
    /// shortest path that still names the one file.
    Shortest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultLinkStyle {
    pub style: LinkStyle,
    pub path_form: LinkPathForm,
}

/// How many note links and attachment embeds a vault already writes in each
/// form, counted note by note by [`count_link_forms`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkFormCounts {
    pub wikilinks: usize,
    pub markdown: usize,
}

/// How recently a note may have changed and still have its counts kept. A
/// second write of the same length inside one timestamp tick would otherwise
/// look unchanged, so a note this fresh is counted again next time.
const SETTLE: Duration = Duration::from_secs(2);

/// A note's counts, kept while its size and modification time stay the same.
struct CountedNote {
    modified: SystemTime,
    len: u64,
    counts: LinkFormCounts,
}

/// Every counted note, by absolute path, across all Vaults. Without it each
/// Vault listing would read every note of every Vault that lacks Obsidian
/// settings; with it a listing re-reads only the notes that changed.
fn counted_notes() -> &'static Mutex<HashMap<PathBuf, CountedNote>> {
    static NOTES: OnceLock<Mutex<HashMap<PathBuf, CountedNote>>> = OnceLock::new();
    NOTES.get_or_init(Default::default)
}

/// The link forms written by the notes of the Vault at `root`, `notes` being
/// all of them. A note that cannot be read counts nothing. Counts kept for a
/// note under `root` that is no longer in `notes` are dropped.
pub fn count_link_forms<'a>(
    root: &Path,
    notes: impl IntoIterator<Item = &'a Path>,
) -> LinkFormCounts {
    let mut counted = counted_notes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut total = LinkFormCounts::default();
    let mut seen = std::collections::HashSet::new();
    for path in notes {
        seen.insert(path.to_path_buf());
        let Ok(metadata) = fs::metadata(path) else {
            continue;
        };
        let (Ok(modified), len) = (metadata.modified(), metadata.len()) else {
            continue;
        };
        let fresh = counted
            .get(path)
            .filter(|note| note.modified == modified && note.len == len)
            .map(|note| note.counts);
        let counts = match fresh {
            Some(counts) => counts,
            None => {
                let Ok(content) = fs::read_to_string(path) else {
                    continue;
                };
                let counts = super::links::note_link_forms(&content);
                let settled = SystemTime::now()
                    .duration_since(modified)
                    .is_ok_and(|age| age >= SETTLE);
                if !settled {
                    total.wikilinks += counts.wikilinks;
                    total.markdown += counts.markdown;
                    continue;
                }
                counted.insert(
                    path.to_path_buf(),
                    CountedNote {
                        modified,
                        len,
                        counts,
                    },
                );
                counts
            }
        };
        total.wikilinks += counts.wikilinks;
        total.markdown += counts.markdown;
    }
    counted.retain(|path, _| !path.starts_with(root) || seen.contains(path));
    total
}

/// The vault's link style. `counts` is consulted only when the vault has no
/// Obsidian settings file, because producing it walks the Vault. `None` when
/// the vault cannot be read at all.
pub fn vault_link_style(
    root: &Path,
    counts: impl FnOnce() -> Option<LinkFormCounts>,
) -> Option<VaultLinkStyle> {
    match obsidian_link_style(root) {
        Some(style) => Some(style),
        None => counts().map(|counts| VaultLinkStyle {
            style: if counts.markdown > counts.wikilinks {
                LinkStyle::Markdown
            } else {
                LinkStyle::Wikilink
            },
            path_form: LinkPathForm::Relative,
        }),
    }
}

/// The style Obsidian recorded, or `None` when the vault has no
/// `.obsidian/app.json`. A file that exists but cannot be read or parsed
/// still means Obsidian owns the choice, so it answers with Obsidian's
/// defaults: wikilinks, shortest paths.
fn obsidian_link_style(root: &Path) -> Option<VaultLinkStyle> {
    let raw = match fs::read_to_string(root.join(OBSIDIAN_APP_CONFIG)) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return None,
        // A Vault directory that cannot be listed is unreadable, not a
        // settings file Hatchdoor failed to parse.
        Err(_) if fs::read_dir(root).is_err() => return None,
        Err(_) => String::new(),
    };
    let settings: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
    let style = if settings
        .get("useMarkdownLinks")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        LinkStyle::Markdown
    } else {
        LinkStyle::Wikilink
    };
    let path_form = match settings
        .get("newLinkFormat")
        .and_then(serde_json::Value::as_str)
    {
        Some("relative") => LinkPathForm::Relative,
        Some("absolute") => LinkPathForm::Absolute,
        _ => LinkPathForm::Shortest,
    };
    Some(VaultLinkStyle { style, path_form })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault_with_app_json(contents: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join(".obsidian")).expect("obsidian dir");
        fs::write(dir.path().join(OBSIDIAN_APP_CONFIG), contents).expect("app.json");
        dir
    }

    fn mostly_markdown() -> Option<LinkFormCounts> {
        Some(LinkFormCounts {
            wikilinks: 1,
            markdown: 5,
        })
    }

    fn style(root: &Path, counts: Option<LinkFormCounts>) -> Option<VaultLinkStyle> {
        vault_link_style(root, || counts)
    }

    #[test]
    fn obsidian_setting_wins_over_the_vaults_own_links() {
        let dir = vault_with_app_json(r#"{"useMarkdownLinks": true, "newLinkFormat": "absolute"}"#);
        let mostly_wikilinks = Some(LinkFormCounts {
            wikilinks: 9,
            markdown: 0,
        });
        assert_eq!(
            style(dir.path(), mostly_wikilinks),
            Some(VaultLinkStyle {
                style: LinkStyle::Markdown,
                path_form: LinkPathForm::Absolute,
            })
        );
    }

    #[test]
    fn obsidian_defaults_apply_for_missing_keys_false_and_garbage() {
        for contents in [
            "{}",
            r#"{"useMarkdownLinks": false}"#,
            "not json {",
            r#"{"useMarkdownLinks": "yes", "newLinkFormat": "weird"}"#,
        ] {
            let dir = vault_with_app_json(contents);
            assert_eq!(
                style(dir.path(), mostly_markdown()),
                Some(VaultLinkStyle {
                    style: LinkStyle::Wikilink,
                    path_form: LinkPathForm::Shortest,
                }),
                "{contents}"
            );
        }
    }

    #[test]
    fn every_obsidian_path_form_is_read() {
        for (value, form) in [
            ("relative", LinkPathForm::Relative),
            ("absolute", LinkPathForm::Absolute),
            ("shortest", LinkPathForm::Shortest),
        ] {
            let dir = vault_with_app_json(&format!(
                r#"{{"useMarkdownLinks": true, "newLinkFormat": "{value}"}}"#
            ));
            assert_eq!(style(dir.path(), None).map(|s| s.path_form), Some(form));
        }
    }

    #[test]
    fn without_obsidian_settings_the_majority_decides_and_ties_go_to_wikilinks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cases = [
            (0, 0, LinkStyle::Wikilink),
            (2, 2, LinkStyle::Wikilink),
            (3, 2, LinkStyle::Wikilink),
            (2, 3, LinkStyle::Markdown),
        ];
        for (wikilinks, markdown, expected) in cases {
            assert_eq!(
                style(
                    dir.path(),
                    Some(LinkFormCounts {
                        wikilinks,
                        markdown
                    })
                ),
                Some(VaultLinkStyle {
                    style: expected,
                    path_form: LinkPathForm::Relative,
                }),
                "{wikilinks} wikilinks, {markdown} markdown"
            );
        }
    }

    #[test]
    fn a_settings_path_that_cannot_be_read_as_a_file_means_obsidian_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join(OBSIDIAN_APP_CONFIG)).expect("a directory in its place");
        assert_eq!(
            style(dir.path(), mostly_markdown()),
            Some(VaultLinkStyle {
                style: LinkStyle::Wikilink,
                path_form: LinkPathForm::Shortest,
            })
        );
    }

    #[test]
    fn an_unreadable_vault_has_no_style() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(style(&dir.path().join("missing"), None), None);
    }

    /// Writes `content` to `path` with a modification time old enough for its
    /// counts to be kept.
    fn write_settled(path: &Path, content: &str) {
        fs::write(path, content).expect("note");
        let old = SystemTime::now() - Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(path)
            .and_then(|file| file.set_modified(old))
            .expect("backdate");
    }

    fn kept(path: &Path) -> Option<LinkFormCounts> {
        let counted = counted_notes().lock().expect("lock");
        counted.get(path).map(|note| note.counts)
    }

    #[test]
    fn counts_are_kept_until_a_note_changes_and_dropped_once_it_is_gone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("A.md");
        let b = dir.path().join("B.md");
        write_settled(&a, "[[B]] ![[pic.png]] `[[code]]`");
        write_settled(&b, "[a](A.md) ![](pic.png) ![](https://example.com/x.png)");
        let count = |paths: &[&Path]| count_link_forms(dir.path(), paths.iter().copied());

        assert_eq!(
            count(&[&a, &b]),
            LinkFormCounts {
                wikilinks: 2,
                markdown: 2,
            }
        );

        // A kept count is used without reading the note: plant a wrong one
        // and it comes back.
        let planted = LinkFormCounts {
            wikilinks: 40,
            markdown: 0,
        };
        counted_notes()
            .lock()
            .expect("lock")
            .get_mut(&a)
            .expect("A is kept")
            .counts = planted;
        assert_eq!(count(&[&a]).wikilinks, 40);

        // A changed note is read again.
        write_settled(&a, "[[B]] [[B]] [[B]]");
        assert_eq!(kept(&a).map(|counts| counts.wikilinks), Some(40));
        assert_eq!(count(&[&a]).wikilinks, 3);

        // B left the listing above, so its counts went with it.
        assert_eq!(kept(&b), None);
    }

    #[test]
    fn a_note_written_just_now_is_counted_but_not_kept() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("A.md");
        fs::write(&a, "[[B]]").expect("note");
        assert_eq!(count_link_forms(dir.path(), [a.as_path()]).wikilinks, 1);
        assert_eq!(kept(&a), None);
    }

    #[test]
    fn the_counts_are_not_taken_when_obsidian_answers() {
        let dir = vault_with_app_json("{}");
        let found = vault_link_style(dir.path(), || panic!("counted links needlessly"));
        assert!(found.is_some());
    }
}
