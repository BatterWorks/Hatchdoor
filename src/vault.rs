use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteEntry {
    pub title: String,
    pub slug: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub title: String,
    pub slug: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct VaultIndex {
    root: PathBuf,
    by_slug: HashMap<String, NoteEntry>,
    by_title: HashMap<String, String>,
}

impl VaultIndex {
    pub fn build(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        let mut by_slug = HashMap::new();
        let mut by_title = HashMap::new();

        for entry in WalkDir::new(&root) {
            let entry = entry.map_err(io::Error::other)?;
            let path = entry.path();

            if !entry.file_type().is_file()
                || path.extension().and_then(|ext| ext.to_str()) != Some("md")
            {
                continue;
            }

            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_string();

            let mut slug = slugify(&stem);
            if slug.is_empty() {
                slug = "untitled".to_string();
            }

            let slug = unique_slug(&slug, &by_slug);
            let note = NoteEntry {
                title: stem.clone(),
                slug: slug.clone(),
                path: path.to_path_buf(),
            };

            by_title.insert(normalize_title(&stem), slug.clone());
            by_slug.insert(slug, note);
        }

        Ok(Self {
            root,
            by_slug,
            by_title,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn find_by_slug(&self, slug: &str) -> Option<&NoteEntry> {
        self.by_slug.get(slug)
    }

    pub fn resolve_wikilink(&self, raw_target: &str) -> Option<&NoteEntry> {
        let normalized_target = strip_md_extension(raw_target.trim());
        let base = normalized_target
            .rsplit('/')
            .next()
            .unwrap_or(normalized_target);

        if let Some(slug) = self.by_title.get(&normalize_title(base)) {
            return self.by_slug.get(slug);
        }

        self.by_slug.get(&slugify(base))
    }

    pub fn read_note_by_slug(&self, slug: &str) -> io::Result<Option<Note>> {
        let Some(entry) = self.find_by_slug(slug) else {
            return Ok(None);
        };

        let content = fs::read_to_string(&entry.path)?;
        Ok(Some(Note {
            title: entry.title.clone(),
            slug: entry.slug.clone(),
            content,
        }))
    }

    #[cfg(test)]
    pub fn total_notes(&self) -> usize {
        self.by_slug.len()
    }
}

fn unique_slug(base: &str, by_slug: &HashMap<String, NoteEntry>) -> String {
    if !by_slug.contains_key(base) {
        return base.to_string();
    }

    let mut idx = 2usize;
    loop {
        let candidate = format!("{base}-{idx}");
        if !by_slug.contains_key(&candidate) {
            return candidate;
        }
        idx += 1;
    }
}

pub fn normalize_title(input: &str) -> String {
    input.trim().to_lowercase()
}

pub fn strip_md_extension(input: &str) -> &str {
    input.strip_suffix(".md").unwrap_or(input)
}

pub fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;

    for c in input.trim().chars() {
        let mapped = if c.is_ascii_alphanumeric() {
            c.to_ascii_lowercase()
        } else if c == ' ' || c == '-' || c == '_' {
            '-'
        } else {
            continue;
        };

        if mapped == '-' {
            if !prev_dash && !out.is_empty() {
                out.push('-');
            }
            prev_dash = true;
        } else {
            out.push(mapped);
            prev_dash = false;
        }
    }

    while out.ends_with('-') {
        out.pop();
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn normalize_title_trims_and_lowercases() {
        assert_eq!(normalize_title("  My NOTE  "), "my note");
    }

    #[test]
    fn strip_md_extension_only_removes_suffix() {
        assert_eq!(strip_md_extension("Note.md"), "Note");
        assert_eq!(strip_md_extension("Note.markdown"), "Note.markdown");
    }

    #[test]
    fn slugify_reduces_symbols_to_clean_slug() {
        assert_eq!(slugify("  My Great_Note!!  "), "my-great-note");
    }

    #[test]
    fn build_indexes_markdown_files_only() {
        let dir = tempdir().expect("temp dir");
        fs::write(dir.path().join("Home.md"), "# Home").expect("write note");
        fs::write(dir.path().join("readme.txt"), "ignore").expect("write text");

        let vault = VaultIndex::build(dir.path()).expect("build vault");
        assert_eq!(vault.total_notes(), 1);
        assert!(vault.resolve_wikilink("Home").is_some());
    }

    #[test]
    fn resolve_wikilink_supports_title_slug_and_md_suffix() {
        let dir = tempdir().expect("temp dir");
        fs::write(dir.path().join("Second Note.md"), "content").expect("write note");

        let vault = VaultIndex::build(dir.path()).expect("build vault");
        assert!(vault.resolve_wikilink("Second Note").is_some());
        assert!(vault.resolve_wikilink("second-note").is_some());
        assert!(vault.resolve_wikilink("Second Note.md").is_some());
    }

    #[test]
    fn duplicate_titles_get_unique_slugs() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir(dir.path().join("a")).expect("create dir a");
        fs::create_dir(dir.path().join("b")).expect("create dir b");
        fs::write(dir.path().join("a").join("Note.md"), "first").expect("write first");
        fs::write(dir.path().join("b").join("Note.md"), "second").expect("write second");

        let vault = VaultIndex::build(dir.path()).expect("build vault");
        assert!(vault.find_by_slug("note").is_some());
        assert!(vault.find_by_slug("note-2").is_some());
    }

    #[test]
    fn read_note_by_slug_reads_existing_and_handles_missing() {
        let dir = tempdir().expect("temp dir");
        fs::write(dir.path().join("Home.md"), "hello").expect("write note");
        let vault = VaultIndex::build(dir.path()).expect("build vault");

        let note = vault
            .read_note_by_slug("home")
            .expect("read success")
            .expect("note exists");
        assert_eq!(note.title, "Home");
        assert_eq!(note.content, "hello");

        let missing = vault.read_note_by_slug("missing").expect("read success");
        assert!(missing.is_none());
    }
}
