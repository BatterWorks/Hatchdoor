use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteEntry {
    pub title: String,
    pub slug: String,
    pub path: PathBuf,
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    pub title: String,
    pub slug: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct VaultIndex {
    by_slug: HashMap<String, NoteEntry>,
    by_title: HashMap<String, String>,
    by_path_title: HashMap<String, String>,
    ordered_slugs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExplorerFolder {
    pub name: String,
    pub folders: Vec<ExplorerFolder>,
    pub notes: Vec<ExplorerNote>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExplorerNote {
    pub title: String,
    pub slug: String,
}

impl VaultIndex {
    pub fn build(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        let mut by_slug = HashMap::new();
        let mut by_title = HashMap::new();
        let mut by_path_title = HashMap::new();
        let mut ordered_slugs = Vec::new();
        let mut markdown_paths = Vec::new();

        for entry in WalkDir::new(&root) {
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

        for path in markdown_paths {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_string();

            let relative_without_ext =
                relative_note_path_without_ext(&root, &path).unwrap_or_else(|| stem.clone());

            let mut slug = slugify(&stem);
            if slug.is_empty() {
                slug = "untitled".to_string();
            }

            let slug = unique_slug(&slug, &by_slug);
            let note = NoteEntry {
                title: stem.clone(),
                slug: slug.clone(),
                path: path.to_path_buf(),
                relative_path: relative_without_ext.clone(),
            };

            by_title
                .entry(normalize_title(&stem))
                .or_insert_with(|| slug.clone());
            by_path_title.insert(normalize_title(&relative_without_ext), slug.clone());
            by_slug.insert(slug, note);
            ordered_slugs.push(relative_without_ext);
        }

        ordered_slugs.sort();
        let ordered_slugs = ordered_slugs
            .into_iter()
            .filter_map(|relative| by_path_title.get(&normalize_title(&relative)).cloned())
            .collect();

        Ok(Self {
            by_slug,
            by_title,
            by_path_title,
            ordered_slugs,
        })
    }

    pub fn find_by_slug(&self, slug: &str) -> Option<&NoteEntry> {
        self.by_slug.get(slug)
    }

    pub fn resolve_wikilink(&self, raw_target: &str) -> Option<&NoteEntry> {
        let normalized_target = normalize_link_target(raw_target);

        if let Some(slug) = self.by_path_title.get(&normalize_title(&normalized_target)) {
            return self.by_slug.get(slug);
        }

        let base = normalized_target
            .rsplit('/')
            .next()
            .unwrap_or(&normalized_target);

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

    pub fn explorer_tree(&self) -> ExplorerFolder {
        let mut root = FolderBuilder::default();

        for slug in &self.ordered_slugs {
            let Some(note) = self.by_slug.get(slug) else {
                continue;
            };
            let mut segments: Vec<&str> = note.relative_path.split('/').collect();
            if segments.is_empty() {
                continue;
            }
            segments.pop();
            root.insert_note(
                &segments,
                ExplorerNote {
                    title: note.title.clone(),
                    slug: note.slug.clone(),
                },
            );
        }

        root.build("Vault")
    }

    #[cfg(test)]
    pub fn total_notes(&self) -> usize {
        self.by_slug.len()
    }
}

#[derive(Default)]
struct FolderBuilder {
    folders: BTreeMap<String, FolderBuilder>,
    notes: Vec<ExplorerNote>,
}

impl FolderBuilder {
    fn insert_note(&mut self, folders: &[&str], note: ExplorerNote) {
        if folders.is_empty() {
            self.notes.push(note);
            return;
        }

        let head = folders[0].to_string();
        self.folders
            .entry(head)
            .or_default()
            .insert_note(&folders[1..], note);
    }

    fn build(self, name: &str) -> ExplorerFolder {
        ExplorerFolder {
            name: name.to_string(),
            folders: self
                .folders
                .into_iter()
                .map(|(folder_name, builder)| builder.build(&folder_name))
                .collect(),
            notes: self.notes,
        }
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

pub fn normalize_link_target(input: &str) -> String {
    strip_md_extension(input.trim()).replace('\\', "/")
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

fn relative_note_path_without_ext(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let as_string = relative.to_str()?.replace('\\', "/");
    Some(strip_md_extension(&as_string).to_string())
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
    fn normalize_link_target_strips_md_and_normalizes_separators() {
        assert_eq!(
            normalize_link_target(r"Folder\My Note.md"),
            "Folder/My Note"
        );
    }

    #[test]
    fn build_indexes_markdown_files_only() {
        let dir = tempdir().expect("temp dir");
        fs::write(dir.path().join("Home.md"), "# Home").expect("write note");
        fs::write(dir.path().join("readme.txt"), "ignore").expect("write text");

        let vault = VaultIndex::build(dir.path()).expect("build vault");
        assert_eq!(vault.total_notes(), 1);
        assert!(vault.resolve_wikilink("Home").is_some());
        let found = vault.find_by_slug("home").expect("home by slug");
        assert_eq!(found.relative_path, "Home");
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
    fn resolve_wikilink_duplicate_title_uses_deterministic_first_path() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir(dir.path().join("b")).expect("create dir b");
        fs::create_dir(dir.path().join("a")).expect("create dir a");
        fs::write(dir.path().join("b").join("Note.md"), "b").expect("write b");
        fs::write(dir.path().join("a").join("Note.md"), "a").expect("write a");

        let vault = VaultIndex::build(dir.path()).expect("build vault");
        let resolved = vault.resolve_wikilink("Note").expect("note exists");
        assert_eq!(resolved.path, dir.path().join("a").join("Note.md"));
    }

    #[test]
    fn resolve_wikilink_supports_folder_qualified_targets() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir(dir.path().join("folder")).expect("create folder");
        fs::write(dir.path().join("folder").join("Doc.md"), "content").expect("write doc");

        let vault = VaultIndex::build(dir.path()).expect("build vault");
        let resolved = vault
            .resolve_wikilink("folder/Doc")
            .expect("folder-qualified match");
        assert_eq!(resolved.path, dir.path().join("folder").join("Doc.md"));
    }

    #[test]
    fn explorer_tree_preserves_folder_structure() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("projects/rust")).expect("create nested folders");
        fs::write(dir.path().join("Home.md"), "home").expect("write home");
        fs::write(dir.path().join("projects").join("Plan.md"), "plan").expect("write plan");
        fs::write(dir.path().join("projects/rust").join("Notes.md"), "notes").expect("write notes");

        let vault = VaultIndex::build(dir.path()).expect("build vault");
        let tree = vault.explorer_tree();

        assert_eq!(tree.name, "Vault");
        assert_eq!(tree.notes.len(), 1);
        assert_eq!(tree.notes[0].title, "Home");
        assert_eq!(tree.folders.len(), 1);
        assert_eq!(tree.folders[0].name, "projects");
        assert_eq!(tree.folders[0].notes[0].title, "Plan");
        assert_eq!(tree.folders[0].folders[0].name, "rust");
        assert_eq!(tree.folders[0].folders[0].notes[0].title, "Notes");
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
