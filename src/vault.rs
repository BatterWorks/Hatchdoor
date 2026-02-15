use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
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
    pub relative_path: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct VaultIndex {
    by_slug: HashMap<String, NoteEntry>,
    by_title: HashMap<String, String>,
    by_path_title: HashMap<String, String>,
    ordered_slugs: Vec<String>,
    outgoing_by_slug: HashMap<String, Vec<String>>,
    backlinks_by_slug: HashMap<String, Vec<String>>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    pub title: String,
    pub slug: String,
    pub relative_path: String,
    pub match_kind: String,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteLink {
    pub title: String,
    pub slug: String,
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteLinks {
    pub outgoing: Vec<NoteLink>,
    pub backlinks: Vec<NoteLink>,
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
            by_path_title
                .entry(normalize_title(&relative_without_ext))
                .or_insert_with(|| slug.clone());
            by_slug.insert(slug.clone(), note);
            ordered_slugs.push(slug);
        }

        ordered_slugs.sort_by(|left_slug, right_slug| {
            let left = by_slug
                .get(left_slug)
                .map(|entry| entry.relative_path.as_str())
                .unwrap_or("");
            let right = by_slug
                .get(right_slug)
                .map(|entry| entry.relative_path.as_str())
                .unwrap_or("");
            left.cmp(right)
        });

        let (outgoing_by_slug, backlinks_by_slug) =
            build_link_graph(&by_slug, &by_title, &by_path_title, &ordered_slugs);

        Ok(Self {
            by_slug,
            by_title,
            by_path_title,
            ordered_slugs,
            outgoing_by_slug,
            backlinks_by_slug,
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
            relative_path: entry.relative_path.clone(),
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

    pub fn search(&self, query: &str, include_content: bool, limit: usize) -> Vec<SearchHit> {
        let normalized_query = normalize_title(query);
        if normalized_query.is_empty() || limit == 0 {
            return Vec::new();
        }

        let mut results = Vec::new();
        let mut seen = HashSet::new();

        for slug in &self.ordered_slugs {
            let Some(note) = self.by_slug.get(slug) else {
                continue;
            };

            let normalized_title = normalize_title(&note.title);
            let normalized_path = normalize_title(&note.relative_path);

            let match_kind = if normalized_title.contains(&normalized_query) {
                Some("title")
            } else if normalized_path.contains(&normalized_query) {
                Some("path")
            } else {
                None
            };

            if let Some(kind) = match_kind {
                seen.insert(note.slug.clone());
                results.push(SearchHit {
                    title: note.title.clone(),
                    slug: note.slug.clone(),
                    relative_path: note.relative_path.clone(),
                    match_kind: kind.to_string(),
                    snippet: None,
                });
                if results.len() >= limit {
                    return results;
                }
            }
        }

        if !include_content {
            return results;
        }

        for slug in &self.ordered_slugs {
            if results.len() >= limit {
                break;
            }

            let Some(note) = self.by_slug.get(slug) else {
                continue;
            };

            if seen.contains(&note.slug) {
                continue;
            }

            let Ok(content) = fs::read_to_string(&note.path) else {
                continue;
            };

            let Some(snippet) = content_snippet(&content, &normalized_query) else {
                continue;
            };

            results.push(SearchHit {
                title: note.title.clone(),
                slug: note.slug.clone(),
                relative_path: note.relative_path.clone(),
                match_kind: "content".to_string(),
                snippet: Some(snippet),
            });
        }

        results
    }

    pub fn note_links(&self, slug: &str) -> Option<NoteLinks> {
        if !self.by_slug.contains_key(slug) {
            return None;
        }

        let outgoing = self
            .outgoing_by_slug
            .get(slug)
            .map(|links| self.map_slugs_to_links(links))
            .unwrap_or_default();
        let backlinks = self
            .backlinks_by_slug
            .get(slug)
            .map(|links| self.map_slugs_to_links(links))
            .unwrap_or_default();

        Some(NoteLinks {
            outgoing,
            backlinks,
        })
    }

    fn map_slugs_to_links(&self, slugs: &[String]) -> Vec<NoteLink> {
        slugs
            .iter()
            .filter_map(|item_slug| {
                self.by_slug.get(item_slug).map(|entry| NoteLink {
                    title: entry.title.clone(),
                    slug: entry.slug.clone(),
                    relative_path: entry.relative_path.clone(),
                })
            })
            .collect()
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

fn content_snippet(content: &str, normalized_query: &str) -> Option<String> {
    content
        .lines()
        .find(|line| normalize_title(line).contains(normalized_query))
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.chars().count() > 180 {
                let shortened: String = trimmed.chars().take(177).collect();
                format!("{shortened}...")
            } else {
                trimmed.to_string()
            }
        })
}

fn build_link_graph(
    by_slug: &HashMap<String, NoteEntry>,
    by_title: &HashMap<String, String>,
    by_path_title: &HashMap<String, String>,
    ordered_slugs: &[String],
) -> (HashMap<String, Vec<String>>, HashMap<String, Vec<String>>) {
    let mut outgoing_by_slug: HashMap<String, Vec<String>> = HashMap::new();
    let mut backlinks_by_slug: HashMap<String, Vec<String>> = HashMap::new();

    for slug in ordered_slugs {
        outgoing_by_slug.insert(slug.clone(), Vec::new());
        backlinks_by_slug.insert(slug.clone(), Vec::new());
    }

    for slug in ordered_slugs {
        let Some(note) = by_slug.get(slug) else {
            continue;
        };

        let Ok(content) = fs::read_to_string(&note.path) else {
            continue;
        };

        let mut seen = HashSet::new();
        let mut outgoing = Vec::new();

        for target in extract_wikilink_targets(&content) {
            let Some(resolved_slug) =
                resolve_target_slug(&target, by_slug, by_title, by_path_title)
            else {
                continue;
            };

            if resolved_slug == note.slug || !seen.insert(resolved_slug.clone()) {
                continue;
            }

            outgoing.push(resolved_slug.clone());
            backlinks_by_slug
                .entry(resolved_slug)
                .or_default()
                .push(note.slug.clone());
        }

        outgoing_by_slug.insert(note.slug.clone(), outgoing);
    }

    for links in outgoing_by_slug.values_mut() {
        sort_slug_links(links, by_slug);
    }
    for links in backlinks_by_slug.values_mut() {
        links.sort();
        links.dedup();
        sort_slug_links(links, by_slug);
    }

    (outgoing_by_slug, backlinks_by_slug)
}

fn sort_slug_links(links: &mut [String], by_slug: &HashMap<String, NoteEntry>) {
    links.sort_by(|left, right| {
        let left_path = by_slug
            .get(left)
            .map(|entry| entry.relative_path.as_str())
            .unwrap_or("");
        let right_path = by_slug
            .get(right)
            .map(|entry| entry.relative_path.as_str())
            .unwrap_or("");
        left_path.cmp(right_path)
    });
}

fn extract_wikilink_targets(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let mut idx = 0usize;
    let mut targets = Vec::new();

    while idx + 1 < bytes.len() {
        if bytes[idx] == b'[' && bytes[idx + 1] == b'[' {
            let is_embed = idx > 0 && bytes[idx - 1] == b'!';
            let mut end = idx + 2;

            while end + 1 < bytes.len() {
                if bytes[end] == b']' && bytes[end + 1] == b']' {
                    break;
                }
                end += 1;
            }

            if end + 1 >= bytes.len() {
                break;
            }

            if !is_embed {
                let body = &content[idx + 2..end];
                let target = parse_wikilink_target(body);
                if !target.is_empty() {
                    targets.push(target);
                }
            }

            idx = end + 2;
            continue;
        }

        idx += 1;
    }

    targets
}

fn parse_wikilink_target(body: &str) -> String {
    let before_alias = body.split('|').next().unwrap_or(body).trim();
    let before_heading = before_alias
        .split('#')
        .next()
        .unwrap_or(before_alias)
        .trim();
    before_heading
        .split('^')
        .next()
        .unwrap_or(before_heading)
        .trim()
        .to_string()
}

fn resolve_target_slug(
    raw_target: &str,
    by_slug: &HashMap<String, NoteEntry>,
    by_title: &HashMap<String, String>,
    by_path_title: &HashMap<String, String>,
) -> Option<String> {
    let normalized_target = normalize_link_target(raw_target);

    if let Some(slug) = by_path_title.get(&normalize_title(&normalized_target)) {
        return Some(slug.clone());
    }

    let base = normalized_target
        .rsplit('/')
        .next()
        .unwrap_or(&normalized_target);

    if let Some(slug) = by_title.get(&normalize_title(base)) {
        return Some(slug.clone());
    }

    let slug = slugify(base);
    if by_slug.contains_key(&slug) {
        return Some(slug);
    }

    None
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
        assert_eq!(note.relative_path, "Home");
        assert_eq!(note.content, "hello");

        let missing = vault.read_note_by_slug("missing").expect("read success");
        assert!(missing.is_none());
    }

    #[test]
    fn search_finds_title_and_path_matches() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("Projects")).expect("create dir");
        fs::write(dir.path().join("Projects").join("Plan.md"), "alpha").expect("write plan");

        let vault = VaultIndex::build(dir.path()).expect("build vault");
        let by_title = vault.search("plan", false, 10);
        assert_eq!(by_title.len(), 1);
        assert_eq!(by_title[0].match_kind, "title");

        let by_path = vault.search("projects", false, 10);
        assert_eq!(by_path.len(), 1);
        assert_eq!(by_path[0].match_kind, "path");
    }

    #[test]
    fn search_content_extension_returns_snippet() {
        let dir = tempdir().expect("temp dir");
        fs::write(
            dir.path().join("Home.md"),
            "Line 1\nSecret token here\nLine 3",
        )
        .expect("write note");

        let vault = VaultIndex::build(dir.path()).expect("build vault");
        let hits = vault.search("token", true, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].match_kind, "content");
        assert_eq!(hits[0].snippet.as_deref(), Some("Secret token here"));
    }

    #[test]
    fn explorer_tree_keeps_notes_with_case_only_path_differences() {
        let dir = tempdir().expect("temp dir");
        fs::write(dir.path().join("Foo.md"), "upper").expect("write upper");
        fs::write(dir.path().join("foo.md"), "lower").expect("write lower");

        let vault = VaultIndex::build(dir.path()).expect("build vault");
        let tree = vault.explorer_tree();

        assert_eq!(tree.notes.len(), 2);
        assert_eq!(tree.notes[0].title, "Foo");
        assert_eq!(tree.notes[1].title, "foo");
    }

    #[test]
    fn note_links_returns_outgoing_and_backlinks() {
        let dir = tempdir().expect("temp dir");
        fs::write(
            dir.path().join("Home.md"),
            "[[Plan]]\n[[Docs/Guide]]\n![[Image.png]]",
        )
        .expect("write home");
        fs::write(dir.path().join("Plan.md"), "[[Home]]").expect("write plan");
        fs::create_dir_all(dir.path().join("Docs")).expect("create docs dir");
        fs::write(dir.path().join("Docs/Guide.md"), "Guide body").expect("write guide");

        let vault = VaultIndex::build(dir.path()).expect("build vault");
        let home_links = vault.note_links("home").expect("home links");

        assert_eq!(home_links.outgoing.len(), 2);
        assert_eq!(home_links.outgoing[0].slug, "guide");
        assert_eq!(home_links.outgoing[1].slug, "plan");
        assert_eq!(home_links.backlinks.len(), 1);
        assert_eq!(home_links.backlinks[0].slug, "plan");

        let guide_links = vault.note_links("guide").expect("guide links");
        assert_eq!(guide_links.backlinks.len(), 1);
        assert_eq!(guide_links.backlinks[0].slug, "home");
        assert!(vault.note_links("missing").is_none());
    }
}
