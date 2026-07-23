use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;

use walkdir::WalkDir;

use super::layers::LayerMap;
use super::links::build_link_graph;
use super::paths::{
    content_snippet, normalize_link_target, normalize_title, relative_note_path_without_ext,
    slugify, unique_slug,
};
use super::types::{
    ExplorerFolder, ExplorerNote, Note, NoteEntry, NoteLink, NoteLinks, SearchHit, VaultIndex,
    VaultScanConfig,
};
use crate::cache::parse::content_hash;

impl VaultIndex {
    /// Scans with default deployment configuration. Retained so existing
    /// callers and tests are unaffected.
    pub fn build(root: impl AsRef<Path>) -> io::Result<Self> {
        Self::build_with_config(root, &VaultScanConfig::default())
    }

    pub fn build_with_config(root: impl AsRef<Path>, config: &VaultScanConfig) -> io::Result<Self> {
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
        // `sort_by_cached_key` is stable, so path order is preserved within each
        // group. Cached rather than plain `sort_by_key` because the key
        // allocates a String: plain would recompute it O(n log n) times on every
        // index build, and every MCP write, HTTP write and watcher refresh
        // rebuilds the index. Cached computes it once per note.
        markdown_paths.sort_by_cached_key(|path| {
            relative_note_path_without_ext(&root, path)
                .map(|relative| layers.layer_for(&relative).is_some())
                .unwrap_or(false)
        });

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
                layer: layers.layer_for(&relative_without_ext).map(str::to_string),
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
            layers,
        })
    }

    pub fn ordered_entries(&self) -> Vec<NoteEntry> {
        self.ordered_slugs
            .iter()
            .filter_map(|slug| self.by_slug.get(slug).cloned())
            .collect()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn find_by_slug(&self, slug: &str) -> Option<&NoteEntry> {
        self.by_slug.get(slug)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn resolve_wikilink(&self, raw_target: &str) -> Option<&NoteEntry> {
        // Strip heading (#) and block (^) anchors — they point within a note, not to a different note
        let note_target = raw_target
            .split('#')
            .next()
            .unwrap_or(raw_target)
            .split('^')
            .next()
            .unwrap_or(raw_target);
        let normalized_target = normalize_link_target(note_target);

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

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn read_note_by_slug(&self, slug: &str) -> io::Result<Option<Note>> {
        let Some(entry) = self.find_by_slug(slug) else {
            return Ok(None);
        };

        let content = fs::read_to_string(&entry.path)?;
        Ok(Some(Note {
            title: entry.title.clone(),
            slug: entry.slug.clone(),
            relative_path: entry.relative_path.clone(),
            content_hash: content_hash(&content),
            content,
            metadata: Default::default(),
        }))
    }

    #[cfg_attr(not(test), allow(dead_code))]
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

    #[cfg_attr(not(test), allow(dead_code))]
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

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Default)]
struct FolderBuilder {
    folders: BTreeMap<String, FolderBuilder>,
    notes: Vec<ExplorerNote>,
}

#[cfg_attr(not(test), allow(dead_code))]
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
