use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::layers::LayerMap;
use super::links::build_link_graph;
use super::markdown_links::{NotePaths, note_link_target};
use super::paths::{
    content_snippet, folder_distance, is_servable_asset, nfc_title, normalize_link_target,
    normalize_title, note_slug_base, relative_note_path_without_ext, resolve_path_ladder, slugify,
    split_wikilink_note_body, unique_slug,
};
use super::types::{
    ExplorerFolder, ExplorerNote, Note, NoteEntry, NoteLink, NoteLinks, NoteMetadata, SearchHit,
    VaultIndex, VaultScanConfig,
};
use crate::cache::parse::{content_hash, parse_frontmatter_metadata};

impl VaultIndex {
    /// Scans with default deployment configuration. Retained so existing
    /// callers and tests are unaffected.
    pub fn build(root: impl AsRef<Path>) -> io::Result<Self> {
        Self::build_with_config(root, &VaultScanConfig::default())
    }

    pub fn build_with_config(root: impl AsRef<Path>, config: &VaultScanConfig) -> io::Result<Self> {
        let mut catalog = Self::build_catalog_with_config(root, config)?;
        let (outgoing_by_slug, backlinks_by_slug) = build_link_graph(&catalog);
        catalog.outgoing_by_slug = outgoing_by_slug;
        catalog.backlinks_by_slug = backlinks_by_slug;
        Ok(catalog)
    }

    /// The metadata-only half of [`Self::build_with_config`]: walks the tree
    /// and assigns slug/title/layer bookkeeping without reading any note's
    /// *content*. Skips [`build_link_graph`] (`links.rs`), which reads and
    /// parses every Markdown file to build the wikilink graph and is the
    /// dominant cost of a full index build in a large Vault.
    ///
    /// A caller that only needs a note's slug or layer — e.g. the write API
    /// filling in its response after a commit already on disk — should use
    /// this instead of `build_with_config` and never pay for the content
    /// read. The returned `VaultIndex`'s `outgoing_by_slug`/`backlinks_by_slug`
    /// are empty; do not call `note_links` on it.
    pub fn build_catalog_with_config(
        root: impl AsRef<Path>,
        config: &VaultScanConfig,
    ) -> io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        let mut by_slug = HashMap::new();
        let mut by_title: HashMap<String, Vec<String>> = HashMap::new();
        let mut by_name_slug: HashMap<String, Vec<String>> = HashMap::new();
        let mut ordered_slugs = Vec::new();
        let mut markdown_paths = Vec::new();
        let mut asset_paths = BTreeSet::new();
        let mut assets_by_name: HashMap<String, Vec<String>> = HashMap::new();
        let mut note_paths = NotePaths::default();

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

            if !entry.file_type().is_file() {
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                // Assets ride the walk the notes already pay for: no extra
                // traversal, and no file is read, so the asset index costs a
                // string per attachment.
                if is_servable_asset(path)
                    && let Some(relative) = relative_asset_path(&root, path)
                {
                    if let Some(name) = relative.rsplit('/').next() {
                        assets_by_name
                            .entry(name.to_lowercase())
                            .or_default()
                            .push(relative.clone());
                    }
                    asset_paths.insert(relative);
                }
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

            let name_slug = note_slug_base(&stem);
            let slug = unique_slug(&name_slug, &by_slug);
            let note = NoteEntry {
                title: stem.clone(),
                slug: slug.clone(),
                path: path.to_path_buf(),
                relative_path: relative_without_ext.clone(),
                layer: layers.layer_for(&relative_without_ext).map(str::to_string),
            };

            by_title
                .entry(normalize_title(&stem))
                .or_default()
                .push(slug.clone());
            by_name_slug
                .entry(name_slug)
                .or_default()
                .push(slug.clone());
            note_paths.insert(&relative_without_ext, &slug);
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

        // Sorted so a name carried by several files resolves deterministically
        // once distance ties, the way duplicate note titles already do.
        for paths in assets_by_name.values_mut() {
            paths.sort();
        }
        note_paths.sort();

        Ok(Self {
            by_slug,
            by_title,
            by_name_slug,
            ordered_slugs,
            asset_paths,
            assets_by_name,
            note_paths,
            outgoing_by_slug: HashMap::new(),
            backlinks_by_slug: HashMap::new(),
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

    /// The note a wikilink target names, read as if written at the Vault
    /// root. See [`Self::resolve_wikilink_match`] for the rule.
    pub fn resolve_wikilink(&self, raw_target: &str) -> Option<&NoteEntry> {
        self.resolve_wikilink_from(raw_target, "")
    }

    /// The note a wikilink written in a note under `note_dir` names. The
    /// folder only breaks ties between namesakes; see
    /// [`Self::resolve_wikilink_match`].
    pub fn resolve_wikilink_from(&self, raw_target: &str, note_dir: &str) -> Option<&NoteEntry> {
        self.resolve_wikilink_match(raw_target, note_dir)
            .map(|found| found.entry)
    }

    /// Resolve a wikilink target, and say whether anything else tied with it.
    ///
    /// Heading (#) and block (^) anchors point within a note and an alias is
    /// display text, so the shared split drops all three, along with the
    /// backslash escaping an alias pipe in a table cell (#252). Target and
    /// names are compared NFC-normalised, so a decomposed filename is reached
    /// by a composed link (#626). In order:
    ///
    /// 1. A target holding `/` names a path: the note whose Vault-relative path
    ///    it is, or ends with as whole segments, so `[[b/Plan]]` reaches
    ///    `notes/b/Plan` and never `notes/a/Plan`.
    /// 2. Otherwise, or when no path matches, the target's last segment is a
    ///    title, compared with case folded.
    /// 3. Failing both, the last segment slugified, matched only against a
    ///    note's own slug. A suffix slug like `meeting-2` is a collision
    ///    counter, not a name, so `[[Meeting 2]]` stays broken (ADR-28).
    ///
    /// Within a tier, a full-path match beats a suffix, an exact-case name
    /// beats a case-folded one, a default-surface note beats a layered one,
    /// and then the note nearest `note_dir` (`""` for the Vault root) wins.
    /// What still ties resolves to
    /// the first note in path order, and `unique` is false.
    pub(crate) fn resolve_wikilink_match(
        &self,
        raw_target: &str,
        note_dir: &str,
    ) -> Option<WikilinkMatch<'_>> {
        let (note_target, _) = split_wikilink_note_body(raw_target);
        let normalized = nfc_title(&normalize_link_target(note_target));
        // A leading `/` anchors the target at the Vault root: only a full
        // path matches it, never a suffix.
        let anchored = normalized.starts_with('/');
        let target = normalized.trim_start_matches('/');
        if target.is_empty() {
            return None;
        }
        let (folder, base) = match target.rsplit_once('/') {
            Some((folder, base)) => (Some(folder), base),
            None => (None, target),
        };
        let distance = |entry: &NoteEntry| folder_distance(note_dir, &entry.relative_path);
        let rank = |entry: &NoteEntry, looser_path: bool, folded_case: bool| WikilinkRank {
            looser_path,
            folded_case,
            layered: entry.layer.is_some(),
            distance: distance(entry),
        };
        let candidates = self.entries_for(&self.by_title, &normalize_title(base));

        if folder.is_some() || anchored {
            let folded = normalize_title(target);
            let by_path = candidates.iter().filter_map(|&entry| {
                let full = normalize_title(&entry.relative_path) == folded;
                if !full && (anchored || !path_ends_with(&entry.relative_path, &folded)) {
                    return None;
                }
                let exact_path = nfc_title(&entry.relative_path);
                let exact = exact_path == target || exact_path.ends_with(&format!("/{target}"));
                Some((entry, rank(entry, !full, !exact)))
            });
            if let Some(found) = best_ranked(by_path) {
                return Some(found);
            }
        }

        let by_title = candidates.iter().map(|&entry| {
            let exact = nfc_title(&entry.title) == base;
            (entry, rank(entry, false, !exact))
        });
        if let Some(found) = best_ranked(by_title) {
            return Some(found);
        }

        // A note reached by its name's slug, never by a collision counter.
        // A folder the target names still decides between namesakes, and
        // only breaks the tie when none sits under it.
        let folder_misses = |entry: &NoteEntry| {
            folder.is_some_and(|folder| {
                let entry_folder = entry
                    .relative_path
                    .rsplit_once('/')
                    .map_or("", |(dir, _)| dir);
                !path_ends_with(entry_folder, &normalize_title(folder))
            })
        };
        let by_slug = self
            .entries_for(&self.by_name_slug, &slugify(base))
            .into_iter()
            .map(|entry| (entry, rank(entry, folder_misses(entry), false)));
        best_ranked(by_slug)
    }

    /// The notes `map` lists under `key`, in claim order.
    fn entries_for(&self, map: &HashMap<String, Vec<String>>, key: &str) -> Vec<&NoteEntry> {
        map.get(key)
            .map(|slugs| {
                slugs
                    .iter()
                    .filter_map(|slug| self.by_slug.get(slug))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// This catalogue as wikilink resolution will read it once the note
    /// `slug` sits at `to`, a Vault-relative path without `.md`. A rename or
    /// move asks it which note each link reaches afterwards, so a link that
    /// would land somewhere new can be pinned to the note it reached before
    /// (#626). Only what resolution reads is moved: the slug, its title and
    /// its layer.
    pub(crate) fn relocated(&self, slug: &str, to: &str) -> Self {
        let mut after = self.clone();
        let Some(entry) = after.by_slug.get_mut(slug) else {
            return after;
        };
        let old_title = normalize_title(&entry.title);
        let old_name_slug = note_slug_base(&entry.title);
        let title = to.rsplit('/').next().unwrap_or(to).to_string();
        entry.layer = self.layers.layer_for(to).map(str::to_string);
        entry.relative_path = to.to_string();
        entry.title = title.clone();
        for (map, old_key, new_key) in [
            (&mut after.by_title, old_title, normalize_title(&title)),
            (
                &mut after.by_name_slug,
                old_name_slug,
                note_slug_base(&title),
            ),
        ] {
            if let Some(slugs) = map.get_mut(&old_key) {
                slugs.retain(|candidate| candidate != slug);
            }
            map.entry(new_key).or_default().push(slug.to_string());
        }
        let by_slug = &after.by_slug;
        // The build's own claim order: default surface first, then the file
        // path compared folder by folder, which is not plain string order
        // when a folder name holds a character sorting before `/`.
        let claim_order = |candidate: &String| {
            by_slug.get(candidate).map(|entry| {
                (
                    entry.layer.is_some(),
                    PathBuf::from(format!("{}.md", entry.relative_path)),
                )
            })
        };
        for map in [&mut after.by_title, &mut after.by_name_slug] {
            for slugs in map.values_mut() {
                slugs.sort_by_cached_key(claim_order);
            }
        }
        after
    }

    /// Resolve a wikilink asset target to a Vault-relative asset path.
    ///
    /// `note_dir` is the `/`-joined Vault-relative directory of the note the
    /// embed was written in, `""` at the Vault root. Order, from most to least
    /// explicit:
    ///
    /// 1. A leading `/` means Vault-root-relative — the one form that is stable
    ///    from any note at any depth.
    /// 2. Relative to the note's own folder, which is what Hatchdoor resolved
    ///    exclusively before (#158) and what `../` forms rely on.
    /// 3. As written from the Vault root, so `98_Attachments/x.png` works from
    ///    a nested note without counting `../`.
    /// 4. By filename anywhere in the Vault, which is Obsidian's "shortest path
    ///    when possible" default and the reason a single attachments folder
    ///    works there. On several matches the one nearest the note wins, so a
    ///    per-folder attachment beats a distant namesake.
    ///
    /// Returns `None` when nothing matches, which lets a caller render a
    /// missing-link affordance instead of emitting a URL that 404s.
    pub fn resolve_asset(&self, raw_target: &str, note_dir: &str) -> Option<&str> {
        let target = raw_target.split(['?', '#']).next().unwrap_or(raw_target);
        let (hit, _) = resolve_path_ladder(
            target,
            note_dir,
            |candidate| self.asset_paths.get(candidate).map(String::as_str),
            |name| {
                // `candidates` is sorted, and `min_by_key` keeps the first of
                // equal keys, so equidistant namesakes resolve by path order
                // rather than by walk order.
                self.assets_by_name
                    .get(&name.to_lowercase())?
                    .iter()
                    .min_by_key(|candidate| folder_distance(note_dir, candidate))
                    .map(String::as_str)
            },
        )?;
        Some(hit)
    }

    /// Resolve the destination of a Markdown note link (ADR-28) to its note.
    ///
    /// `raw_destination` is the destination exactly as written between the
    /// parentheses, percent-escapes and `#anchor` included; anything that is
    /// not a note link (an external URL, a path not ending `.md`) is `None`.
    /// `note_dir` is the linking note's Vault-relative folder. Resolution is
    /// the path ladder attachments use, never the wikilink title rule.
    pub fn resolve_note_link(&self, raw_destination: &str, note_dir: &str) -> Option<&NoteEntry> {
        let target = note_link_target(raw_destination)?;
        let (slug, _) = self.note_paths.resolve(&target.path, note_dir)?;
        self.by_slug.get(slug)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn read_note_by_slug(&self, slug: &str) -> io::Result<Option<Note>> {
        let Some(entry) = self.find_by_slug(slug) else {
            return Ok(None);
        };

        let content = fs::read_to_string(&entry.path)?;
        // The parser the frontmatter read uses, so the two cannot drift. Bad
        // frontmatter leaves the note readable: that is how it gets repaired.
        let metadata = parse_frontmatter_metadata(&content)
            .ok()
            .map(|frontmatter| NoteMetadata {
                tags: frontmatter.tags,
                aliases: frontmatter.aliases,
                properties: serde_json::Value::Object(frontmatter.properties),
            });
        Ok(Some(Note {
            title: entry.title.clone(),
            slug: entry.slug.clone(),
            relative_path: entry.relative_path.clone(),
            content_hash: content_hash(&content),
            content,
            layer: entry.layer.clone(),
            metadata,
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
                    layer: entry.layer.clone(),
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

fn relative_asset_path(root: &Path, path: &Path) -> Option<String> {
    Some(path.strip_prefix(root).ok()?.to_str()?.replace('\\', "/"))
}

/// A wikilink's note, and whether it was the only best match.
pub(crate) struct WikilinkMatch<'a> {
    pub(crate) entry: &'a NoteEntry,
    /// False when another note ranked equally and path order alone chose.
    pub(crate) unique: bool,
}

/// How well one candidate fits a wikilink target, best first. Field order is
/// the ranking: each field breaks only a tie in the fields before it.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct WikilinkRank {
    /// The target's folders matched only the end of the note's path, or, for
    /// a slug, not at all.
    looser_path: bool,
    /// The target matched only once case was folded.
    folded_case: bool,
    /// The note sits in a layer rather than on the default surface.
    layered: bool,
    /// Folder hops from the linking note's folder.
    distance: usize,
}

/// Whether `path` is `folded_tail` or ends with it as whole segments, case
/// and normalisation folded.
fn path_ends_with(path: &str, folded_tail: &str) -> bool {
    let path = normalize_title(path);
    path == folded_tail || path.ends_with(&format!("/{folded_tail}"))
}

/// The lowest-ranked candidate, first in iteration order on a tie, and
/// whether it was alone at that rank.
fn best_ranked<'a>(
    candidates: impl Iterator<Item = (&'a NoteEntry, WikilinkRank)>,
) -> Option<WikilinkMatch<'a>> {
    let mut best: Option<(&NoteEntry, WikilinkRank, bool)> = None;
    for (entry, rank) in candidates {
        match &mut best {
            Some((_, best_rank, unique)) if rank == *best_rank => *unique = false,
            Some((_, best_rank, _)) if rank > *best_rank => {}
            _ => best = Some((entry, rank, true)),
        }
    }
    best.map(|(entry, _, unique)| WikilinkMatch { entry, unique })
}
