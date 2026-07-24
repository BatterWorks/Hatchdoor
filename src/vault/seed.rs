use std::fs;
use std::io;
use std::path::Path;

use walkdir::WalkDir;

use super::exclude::ExcludeMatcher;

const STARTER_NOTES: &[(&str, &str)] = &[
    (
        "README.md",
        include_str!("../../docs/starter-vault/README.md"),
    ),
    (
        "40-reference/Hatchdoor — Getting Started.md",
        include_str!("../../docs/starter-vault/40-reference/Hatchdoor — Getting Started.md"),
    ),
    (
        "40-reference/Hatchdoor — Agent Guide.md",
        include_str!("../../docs/starter-vault/40-reference/Hatchdoor — Agent Guide.md"),
    ),
    (
        "40-reference/Hatchdoor — Agent Skill.md",
        include_str!("../../docs/starter-vault/40-reference/Hatchdoor — Agent Skill.md"),
    ),
    (
        "40-reference/Hatchdoor — Markdown Feature Showcase.md",
        include_str!(
            "../../docs/starter-vault/40-reference/Hatchdoor — Markdown Feature Showcase.md"
        ),
    ),
    (
        "40-reference/Hatchdoor — Starter Vault Organisation.md",
        include_str!(
            "../../docs/starter-vault/40-reference/Hatchdoor — Starter Vault Organisation.md"
        ),
    ),
    (
        "10-topics/Topics Index.md",
        include_str!("../../docs/starter-vault/10-topics/Topics Index.md"),
    ),
    (
        "20-projects/Projects Index.md",
        include_str!("../../docs/starter-vault/20-projects/Projects Index.md"),
    ),
    (
        "30-areas/Areas Index.md",
        include_str!("../../docs/starter-vault/30-areas/Areas Index.md"),
    ),
];

/// Seeds a fresh vault with starter notes when it holds no markdown. `exclude`
/// is the same noise matcher the index build uses, so the "is this vault empty?"
/// decision and the index agree on what counts as content (phase-1 review
/// flagged the earlier default-only divergence).
pub fn seed_empty_vault(root: impl AsRef<Path>, exclude: &ExcludeMatcher) -> io::Result<bool> {
    let root = root.as_ref();
    fs::create_dir_all(root)?;

    if has_markdown_notes(root, exclude)? {
        return Ok(false);
    }

    for (relative_path, content) in STARTER_NOTES {
        let path = root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }

    Ok(true)
}

fn has_markdown_notes(root: &Path, exclude: &ExcludeMatcher) -> io::Result<bool> {
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

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::super::exclude::ExcludeMatcher;
    use super::seed_empty_vault;

    fn matcher(patterns: &[&str]) -> ExcludeMatcher {
        let owned: Vec<String> = patterns.iter().map(|p| p.to_string()).collect();
        ExcludeMatcher::new(&owned).expect("valid patterns")
    }

    #[test]
    fn seeds_starter_notes_when_vault_has_no_markdown_files() {
        let dir = tempdir().expect("temp dir");

        let seeded = seed_empty_vault(dir.path(), &matcher(&[])).expect("seed vault");

        assert!(seeded);
        assert!(dir.path().join("README.md").is_file());
        assert!(
            dir.path()
                .join("40-reference/Hatchdoor — Getting Started.md")
                .is_file()
        );
        assert!(
            dir.path()
                .join("40-reference/Hatchdoor — Agent Skill.md")
                .is_file()
        );
        assert!(dir.path().join("10-topics/Topics Index.md").is_file());
        assert!(dir.path().join("20-projects/Projects Index.md").is_file());
        assert!(dir.path().join("30-areas/Areas Index.md").is_file());
    }

    #[test]
    fn does_not_seed_when_vault_already_has_markdown() {
        let dir = tempdir().expect("temp dir");
        fs::write(dir.path().join("Existing.md"), "# Existing\n").expect("write existing note");

        let seeded = seed_empty_vault(dir.path(), &matcher(&[])).expect("seed vault");

        assert!(!seeded);
        assert!(!dir.path().join("README.md").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("Existing.md")).expect("read existing note"),
            "# Existing\n"
        );
    }

    #[test]
    fn ignores_hatchdoor_trash_when_deciding_if_vault_is_empty() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join(".hatchdoor-trash")).expect("create trash");
        fs::write(
            dir.path().join(".hatchdoor-trash/Deleted.md"),
            "# Deleted\n",
        )
        .expect("write trashed note");

        let seeded = seed_empty_vault(dir.path(), &matcher(&[])).expect("seed vault");

        assert!(seeded);
        assert!(dir.path().join("README.md").is_file());
        assert_eq!(
            fs::read_to_string(dir.path().join(".hatchdoor-trash/Deleted.md"))
                .expect("read trashed note"),
            "# Deleted\n"
        );
    }

    #[test]
    fn seeds_when_the_only_markdown_is_excluded_by_a_user_pattern() {
        // A vault whose only note matches HATCHDOOR_EXCLUDE has no *content*, so
        // the seeder must treat it as empty and seed — using the same matcher the
        // index build uses, not the built-in defaults alone.
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("build")).expect("create build dir");
        fs::write(dir.path().join("build/Generated.md"), "# Generated\n")
            .expect("write generated note");

        let seeded = seed_empty_vault(dir.path(), &matcher(&["build/"])).expect("seed vault");

        assert!(seeded);
        assert!(dir.path().join("README.md").is_file());
        // Without the user pattern the same vault is considered non-empty.
        let dir2 = tempdir().expect("temp dir");
        fs::create_dir_all(dir2.path().join("build")).expect("create build dir");
        fs::write(dir2.path().join("build/Generated.md"), "# Generated\n")
            .expect("write generated note");
        assert!(!seed_empty_vault(dir2.path(), &matcher(&[])).expect("seed vault"));
    }
}
