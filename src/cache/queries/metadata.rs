//! Note metadata, tree, recently-modified, and vault-statistics queries.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params};

use crate::cache::SqliteCache;
use crate::search::LayerSelection;
use crate::vault::{ExplorerFolder, ExplorerNote, ModifiedNote, Note, NoteMetadata, NoteSummary};

impl SqliteCache {
    /// Lightweight liveness probe used by `/health`: confirms the cache database
    /// is reachable rather than just that the process is running.
    pub fn health_check(&self) -> Result<(), String> {
        let conn = self.read()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .map(|_| ())
            .map_err(|error| format!("health check query failed: {error}"))
    }

    pub fn read_note_by_slug(&self, slug: &str) -> Result<Option<Note>, String> {
        let conn = self.read()?;
        let row = conn
            .query_row(
                r#"
            SELECT title, slug, relative_path, content, content_hash,
                   layer, aliases_json, frontmatter_json
            FROM notes
            WHERE slug = ?1
            "#,
                params![slug],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("failed to read note '{slug}' from SQLite cache: {error}"))?;
        let Some((
            title,
            slug,
            relative_path,
            content,
            content_hash,
            layer,
            aliases_json,
            properties_json,
        )) = row
        else {
            return Ok(None);
        };
        let mut tags_stmt = conn
            .prepare("SELECT tag FROM tags WHERE note_slug = ?1 ORDER BY tag")
            .map_err(|error| format!("failed preparing tags for '{slug}': {error}"))?;
        let tags = tags_stmt
            .query_map(params![&slug], |row| row.get::<_, String>(0))
            .map_err(|error| format!("failed querying tags for '{slug}': {error}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("failed reading tags for '{slug}': {error}"))?;
        let aliases = serde_json::from_str(&aliases_json)
            .map_err(|error| format!("invalid cached aliases for '{slug}': {error}"))?;
        let properties = serde_json::from_str(&properties_json)
            .map_err(|error| format!("invalid cached frontmatter for '{slug}': {error}"))?;

        Ok(Some(Note {
            title,
            slug,
            relative_path,
            content,
            content_hash,
            layer,
            metadata: NoteMetadata {
                tags,
                aliases,
                properties,
            },
        }))
    }

    /// Fetch a note by its vault-relative path (with or without a `.md`
    /// extension), reaching any layer. The path is normalized the same way a
    /// wikilink target is, so `sources/Clip`, `sources/Clip.md` and
    /// `Sources/clip` all resolve to the same note. A demoted note stays
    /// reachable by this stable address regardless of the default surface.
    pub fn read_note_by_path(&self, path: &str) -> Result<Option<Note>, String> {
        use crate::vault::{normalize_link_target, normalize_title};
        let normalized = normalize_title(&normalize_link_target(path));
        let conn = self.read()?;
        let slug: Option<String> = conn
            .query_row(
                "SELECT slug FROM notes WHERE normalized_relative_path = ?1 \
                 ORDER BY (layer IS NOT NULL), relative_path LIMIT 1",
                params![normalized],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("failed to resolve note by path '{path}': {error}"))?;
        drop(conn);
        match slug {
            Some(slug) => self.read_note_by_slug(&slug),
            None => Ok(None),
        }
    }

    /// If every note under `path_prefix` sits in demoted layers (no
    /// default-surface note among them), returns those layer names, sorted;
    /// otherwise `None`. Used to turn a `path_prefix` that points wholly inside
    /// unselected demoted layers into an actionable error rather than a silently
    /// empty result. A prefix that also covers a default-surface note, or that
    /// matches nothing, returns `None`, so a mixed or ordinary prefix never
    /// errors — even when it spans more than one demoted layer.
    pub fn demoted_layers_under_prefix(
        &self,
        path_prefix: &str,
    ) -> Result<Option<Vec<String>>, String> {
        let needle = path_prefix.trim().trim_matches('/').to_lowercase();
        if needle.is_empty() {
            return Ok(None);
        }
        let conn = self.read()?;
        let like = format!("{}%", super::search::escape_like(&needle));
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT layer FROM notes WHERE LOWER(relative_path) LIKE ?1 ESCAPE '\\'",
            )
            .map_err(|e| format!("prepare prefix-layer query: {e}"))?;
        let layers: Vec<Option<String>> = stmt
            .query_map(params![like], |row| row.get::<_, Option<String>>(0))
            .map_err(|e| format!("query prefix layers: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("read prefix layers: {e}"))?;
        // A default-surface note under the prefix (or no match at all) means the
        // prefix is not wholly inside demoted space: no error.
        if layers.is_empty() || layers.iter().any(Option::is_none) {
            return Ok(None);
        }
        let mut names: Vec<String> = layers.into_iter().flatten().collect();
        names.sort();
        Ok(Some(names))
    }

    pub fn note_summaries(&self, selection: &LayerSelection) -> Result<Vec<NoteSummary>, String> {
        let conn = self.read()?;
        let mut tags_by_slug: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        let mut tags_stmt = conn
            .prepare("SELECT note_slug, tag FROM tags ORDER BY note_slug, tag")
            .map_err(|error| format!("failed preparing note tags: {error}"))?;
        let tag_rows = tags_stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("failed querying note tags: {error}"))?;
        for row in tag_rows {
            let (slug, tag) = row.map_err(|error| format!("failed reading note tag: {error}"))?;
            tags_by_slug.entry(slug).or_default().push(tag);
        }
        drop(tags_stmt);

        let sql = format!(
            "SELECT title, slug, relative_path, layer, aliases_json, frontmatter_json \
             FROM notes WHERE {} ORDER BY relative_path",
            selection.sql_filter("layer"),
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|error| format!("failed preparing note metadata query: {error}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(|error| format!("failed querying note metadata: {error}"))?;
        let mut notes = Vec::new();
        for row in rows {
            let (title, slug, relative_path, layer, aliases_json, properties_json) =
                row.map_err(|error| format!("failed reading note metadata: {error}"))?;
            notes.push(NoteSummary {
                title,
                metadata: NoteMetadata {
                    tags: tags_by_slug.remove(&slug).unwrap_or_default(),
                    aliases: serde_json::from_str(&aliases_json)
                        .map_err(|error| format!("invalid cached aliases for '{slug}': {error}"))?,
                    properties: serde_json::from_str(&properties_json).map_err(|error| {
                        format!("invalid cached frontmatter for '{slug}': {error}")
                    })?,
                },
                slug,
                relative_path,
                layer,
            });
        }
        Ok(notes)
    }

    pub fn explorer_tree(&self, selection: &LayerSelection) -> Result<ExplorerFolder, String> {
        let rows = self.note_rows_ordered(selection)?;
        let mut root = FolderBuilder::default();

        for row in rows {
            let mut segments: Vec<&str> = row.relative_path.split('/').collect();
            if segments.is_empty() {
                continue;
            }
            segments.pop();
            root.insert_note(
                &segments,
                ExplorerNote {
                    title: row.title,
                    slug: row.slug,
                },
            );
        }

        Ok(root.build("Vault"))
    }

    pub fn recently_modified_notes(
        &self,
        limit: usize,
        selection: &LayerSelection,
    ) -> Result<Vec<ModifiedNote>, String> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let conn = self.read()?;
        let sql = format!(
            r#"
                SELECT title, slug, relative_path, mtime_ns, layer
                FROM notes
                WHERE {}
                ORDER BY mtime_ns DESC, relative_path ASC
                LIMIT ?1
                "#,
            selection.sql_filter("layer"),
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|error| format!("failed to prepare recently modified query: {error}"))?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(ModifiedNote {
                    title: row.get(0)?,
                    slug: row.get(1)?,
                    relative_path: row.get(2)?,
                    mtime_ns: row.get(3)?,
                    layer: row.get(4)?,
                })
            })
            .map_err(|error| format!("failed to query recently modified notes: {error}"))?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("failed to read recently modified notes: {error}"))
    }

    fn note_rows_ordered(&self, selection: &LayerSelection) -> Result<Vec<NoteRow>, String> {
        let conn = self.read()?;
        let sql = format!(
            "SELECT title, slug, relative_path FROM notes WHERE {} ORDER BY relative_path",
            selection.sql_filter("layer"),
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|error| format!("failed to prepare note list query: {error}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(NoteRow {
                    title: row.get(0)?,
                    slug: row.get(1)?,
                    relative_path: row.get(2)?,
                })
            })
            .map_err(|error| format!("failed to query notes from SQLite cache: {error}"))?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("failed to read notes from SQLite cache: {error}"))
    }

    pub fn vault_stats(
        &self,
        selection: &LayerSelection,
    ) -> Result<crate::api_types::VaultStatsResponse, String> {
        use crate::api_types::{
            FolderStat, LinkedNoteRef, MonthActivity, NoteList, NoteRef, NoteWordRef, TagStat,
            VaultStatsResponse,
        };

        let conn = self.read()?;

        // Stats describe the selected surface: every note aggregate is
        // restricted to the selection, and link aggregates only count links
        // whose relevant endpoint(s) are also in the selection. Under the
        // default selection this excludes demoted layers entirely.
        let notes_f = selection.sql_filter("layer");
        let n_f = selection.sql_filter("n.layer");
        let src_f = selection.sql_filter("src.layer");
        let s_f = selection.sql_filter("s.layer");
        let t_f = selection.sql_filter("t.layer");
        let tag_notes_f = selection.sql_filter("notes.layer");

        let note_count: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM notes WHERE {notes_f}"),
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("vault_stats note_count: {e}"))?;

        let tag_count: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(DISTINCT tag) FROM tags \
                     JOIN notes ON notes.slug = tags.note_slug WHERE {tag_notes_f}"
                ),
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("vault_stats tag_count: {e}"))?;

        let link_count: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM note_links l \
                     JOIN notes s ON s.slug = l.source_slug \
                     JOIN notes t ON t.slug = l.target_slug \
                     WHERE {s_f} AND {t_f}"
                ),
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("vault_stats link_count: {e}"))?;

        let vault_size_bytes: i64 = conn
            .query_row(
                &format!("SELECT COALESCE(SUM(size_bytes), 0) FROM notes WHERE {notes_f}"),
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("vault_stats vault_size_bytes: {e}"))?;

        // Fetch all content for word/image count and word-rank computations.
        struct ContentRow {
            slug: String,
            title: String,
            content: String,
        }
        let mut content_stmt = conn
            .prepare(&format!(
                "SELECT slug, title, content FROM notes WHERE {notes_f} ORDER BY relative_path"
            ))
            .map_err(|e| format!("vault_stats prepare content: {e}"))?;
        let content_rows: Vec<ContentRow> = content_stmt
            .query_map([], |row| {
                Ok(ContentRow {
                    slug: row.get(0)?,
                    title: row.get(1)?,
                    content: row.get(2)?,
                })
            })
            .map_err(|e| format!("vault_stats query content: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("vault_stats read content: {e}"))?;
        drop(content_stmt);

        let mut total_word_count: usize = 0;
        let mut total_image_count: usize = 0;
        let mut word_counts: Vec<(String, String, usize)> = Vec::with_capacity(content_rows.len());
        for row in &content_rows {
            let wc = word_count_for_content(&row.content);
            total_word_count += wc;
            total_image_count += row.content.matches("![").count();
            word_counts.push((row.slug.clone(), row.title.clone(), wc));
        }

        let avg_word_count = if note_count > 0 {
            total_word_count / note_count as usize
        } else {
            0
        };

        word_counts.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
        let longest_notes: Vec<NoteWordRef> = word_counts
            .iter()
            .take(5)
            .map(|(slug, title, wc)| NoteWordRef {
                title: title.clone(),
                slug: slug.clone(),
                word_count: *wc,
            })
            .collect();

        word_counts.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)));
        let shortest_notes: Vec<NoteWordRef> = word_counts
            .iter()
            .filter(|(_, _, wc)| *wc > 0)
            .take(5)
            .map(|(slug, title, wc)| NoteWordRef {
                title: title.clone(),
                slug: slug.clone(),
                word_count: *wc,
            })
            .collect();

        let mut tags_stmt = conn
            .prepare(&format!(
                "SELECT tag, COUNT(*) as note_count FROM tags \
                 JOIN notes ON notes.slug = tags.note_slug WHERE {tag_notes_f} \
                 GROUP BY tag ORDER BY note_count DESC, tag LIMIT 20"
            ))
            .map_err(|e| format!("vault_stats prepare top_tags: {e}"))?;
        let top_tags: Vec<TagStat> = tags_stmt
            .query_map([], |row| {
                Ok(TagStat {
                    tag: row.get(0)?,
                    note_count: row.get(1)?,
                })
            })
            .map_err(|e| format!("vault_stats query top_tags: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("vault_stats read top_tags: {e}"))?;
        drop(tags_stmt);

        let mut linked_stmt = conn
            .prepare(&format!(
                r#"
                SELECT n.title, n.slug, COUNT(src.slug) as backlink_count
                FROM notes n
                LEFT JOIN note_links l ON l.target_slug = n.slug
                LEFT JOIN notes src ON src.slug = l.source_slug AND {src_f}
                WHERE {n_f}
                GROUP BY n.slug
                HAVING backlink_count > 0
                ORDER BY backlink_count DESC, n.title
                LIMIT 20
                "#
            ))
            .map_err(|e| format!("vault_stats prepare most_linked: {e}"))?;
        let most_linked: Vec<LinkedNoteRef> = linked_stmt
            .query_map([], |row| {
                Ok(LinkedNoteRef {
                    title: row.get(0)?,
                    slug: row.get(1)?,
                    backlink_count: row.get(2)?,
                })
            })
            .map_err(|e| format!("vault_stats query most_linked: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("vault_stats read most_linked: {e}"))?;
        drop(linked_stmt);

        let mut activity_stmt = conn
            .prepare(&format!(
                r#"
                SELECT strftime('%Y-%m', mtime_ns / 1000000000, 'unixepoch') as month,
                       COUNT(*) as modified_count
                FROM notes
                WHERE {notes_f}
                GROUP BY month
                ORDER BY month DESC
                LIMIT 6
                "#
            ))
            .map_err(|e| format!("vault_stats prepare activity_by_month: {e}"))?;
        let activity_by_month: Vec<MonthActivity> = activity_stmt
            .query_map([], |row| {
                Ok(MonthActivity {
                    month: row.get(0)?,
                    modified_count: row.get(1)?,
                })
            })
            .map_err(|e| format!("vault_stats query activity_by_month: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("vault_stats read activity_by_month: {e}"))?;
        drop(activity_stmt);

        let mut folder_stmt = conn
            .prepare(&format!(
                r#"
                SELECT
                  CASE WHEN instr(relative_path, '/') > 0
                    THEN substr(relative_path, 1, instr(relative_path, '/') - 1)
                    ELSE ''
                  END as folder,
                  COUNT(*) as note_count
                FROM notes
                WHERE {notes_f}
                GROUP BY folder
                ORDER BY note_count DESC, folder
                "#
            ))
            .map_err(|e| format!("vault_stats prepare notes_per_folder: {e}"))?;
        let notes_per_folder: Vec<FolderStat> = folder_stmt
            .query_map([], |row| {
                Ok(FolderStat {
                    folder: row.get(0)?,
                    note_count: row.get(1)?,
                })
            })
            .map_err(|e| format!("vault_stats query notes_per_folder: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("vault_stats read notes_per_folder: {e}"))?;
        drop(folder_stmt);

        // Orphan status is per selection: a selected note is an orphan when it
        // has no link to or from another note that is also in the selection. A
        // default note whose only backlink comes from a demoted source is an
        // orphan under the default selection.
        let mut orphan_stmt = conn
            .prepare(&format!(
                r#"
                SELECT n.title, n.slug FROM notes n
                WHERE {n_f}
                  AND n.slug NOT IN (
                    SELECT l.source_slug FROM note_links l
                    JOIN notes t ON t.slug = l.target_slug WHERE {t_f}
                  )
                  AND n.slug NOT IN (
                    SELECT l.target_slug FROM note_links l
                    JOIN notes s ON s.slug = l.source_slug WHERE {s_f}
                  )
                ORDER BY n.title
                "#
            ))
            .map_err(|e| format!("vault_stats prepare orphan_notes: {e}"))?;
        let orphan_notes: Vec<NoteRef> = orphan_stmt
            .query_map([], |row| {
                Ok(NoteRef {
                    title: row.get(0)?,
                    slug: row.get(1)?,
                })
            })
            .map_err(|e| format!("vault_stats query orphan_notes: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("vault_stats read orphan_notes: {e}"))?;
        drop(orphan_stmt);

        let mut no_tag_stmt = conn
            .prepare(&format!(
                r#"
                SELECT title, slug FROM notes
                WHERE {notes_f}
                  AND slug NOT IN (SELECT DISTINCT note_slug FROM tags)
                ORDER BY title
                "#
            ))
            .map_err(|e| format!("vault_stats prepare no_tag_notes: {e}"))?;
        let no_tag_notes: Vec<NoteRef> = no_tag_stmt
            .query_map([], |row| {
                Ok(NoteRef {
                    title: row.get(0)?,
                    slug: row.get(1)?,
                })
            })
            .map_err(|e| format!("vault_stats query no_tag_notes: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("vault_stats read no_tag_notes: {e}"))?;
        drop(no_tag_stmt);

        let week_total: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM notes \
                     WHERE {notes_f} \
                     AND mtime_ns >= (unixepoch('now') - 7 * 86400) * 1000000000"
                ),
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("vault_stats week_count: {e}"))?;
        let mut week_stmt = conn
            .prepare(&format!(
                r#"
                SELECT title, slug FROM notes
                WHERE {notes_f}
                  AND mtime_ns >= (unixepoch('now') - 7 * 86400) * 1000000000
                ORDER BY mtime_ns DESC
                LIMIT 20
                "#
            ))
            .map_err(|e| format!("vault_stats prepare modified_this_week: {e}"))?;
        let week_notes: Vec<NoteRef> = week_stmt
            .query_map([], |row| {
                Ok(NoteRef {
                    title: row.get(0)?,
                    slug: row.get(1)?,
                })
            })
            .map_err(|e| format!("vault_stats query modified_this_week: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("vault_stats read modified_this_week: {e}"))?;
        drop(week_stmt);

        let month_total: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM notes \
                     WHERE {notes_f} \
                     AND mtime_ns >= (unixepoch('now') - 30 * 86400) * 1000000000"
                ),
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("vault_stats month_count: {e}"))?;
        let mut month_stmt = conn
            .prepare(&format!(
                r#"
                SELECT title, slug FROM notes
                WHERE {notes_f}
                  AND mtime_ns >= (unixepoch('now') - 30 * 86400) * 1000000000
                ORDER BY mtime_ns DESC
                LIMIT 20
                "#
            ))
            .map_err(|e| format!("vault_stats prepare modified_this_month: {e}"))?;
        let month_notes: Vec<NoteRef> = month_stmt
            .query_map([], |row| {
                Ok(NoteRef {
                    title: row.get(0)?,
                    slug: row.get(1)?,
                })
            })
            .map_err(|e| format!("vault_stats query modified_this_month: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("vault_stats read modified_this_month: {e}"))?;
        drop(month_stmt);

        Ok(VaultStatsResponse {
            note_count,
            word_count: total_word_count,
            tag_count,
            link_count,
            image_count: total_image_count,
            avg_word_count,
            vault_size_bytes,
            total_outgoing_links: link_count,
            total_backlinks: link_count,
            top_tags,
            most_linked,
            activity_by_month,
            notes_per_folder,
            longest_notes,
            shortest_notes,
            orphan_notes,
            no_tag_notes,
            modified_this_week: NoteList {
                count: week_total,
                notes: week_notes,
            },
            modified_this_month: NoteList {
                count: month_total,
                notes: month_notes,
            },
        })
    }
}

#[derive(Debug)]
struct NoteRow {
    title: String,
    slug: String,
    relative_path: String,
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

fn word_count_for_content(content: &str) -> usize {
    strip_frontmatter(content).split_whitespace().count()
}

fn strip_frontmatter(content: &str) -> &str {
    let s = content.trim_start_matches('\n');
    let body = match s.strip_prefix("---\n") {
        Some(rest) => rest,
        None => return content,
    };
    if let Some(pos) = body.find("\n---\n") {
        return &body[pos + 5..];
    }
    if let Some(stripped) = body.strip_suffix("\n---") {
        let _ = stripped;
        return "";
    }
    content
}

#[cfg(test)]
mod tests {
    use crate::cache::SqliteCache;
    use crate::embed::StubEmbedder;
    use crate::vault::VaultIndex;
    use rusqlite::params;
    use std::fs;
    use std::sync::Arc;
    use tempfile::tempdir;

    use crate::search::LayerSelection;

    /// Builds a vault with a demoted `sources/` layer and a default-surface
    /// `wiki/` folder, then populates a fresh in-memory cache from it.
    fn build_layered_cache() -> SqliteCache {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("sources")).expect("sources dir");
        fs::create_dir_all(dir.path().join("wiki")).expect("wiki dir");
        fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");
        fs::write(dir.path().join("sources/Clip.md"), "# Clip\nraw source").expect("clip");
        fs::write(dir.path().join("wiki/Page.md"), "# Page\ncompiled").expect("page");

        let cache = SqliteCache::in_memory(384).expect("cache");
        let embedder = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build index");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("populate");
        cache
    }

    #[test]
    fn demoted_layers_under_prefix_reports_all_demoted_layers_and_skips_mixed_prefixes() {
        // Two demoted layers share the `20-` path prefix; a default note lives
        // elsewhere.
        let dir = tempdir().expect("temp dir");
        for (folder, layer) in [("20-sources", "sources"), ("20-archive", "archive")] {
            fs::create_dir_all(dir.path().join(folder)).expect("dir");
            fs::write(dir.path().join(folder).join(".hatchdoor-layer"), layer).expect("marker");
            fs::write(dir.path().join(folder).join("N.md"), "# N").expect("note");
        }
        fs::create_dir_all(dir.path().join("wiki")).expect("wiki dir");
        fs::write(dir.path().join("wiki/Page.md"), "# Page").expect("page");

        let cache = SqliteCache::in_memory(384).expect("cache");
        let embedder = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build index");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("populate");

        // A prefix spanning two demoted layers (no default note) reports both,
        // sorted — so the precedence check can name them instead of returning
        // a silent empty result.
        assert_eq!(
            cache.demoted_layers_under_prefix("20").expect("query"),
            Some(vec!["archive".to_string(), "sources".to_string()])
        );
        // A single demoted layer still reported.
        assert_eq!(
            cache
                .demoted_layers_under_prefix("20-sources")
                .expect("query"),
            Some(vec!["sources".to_string()])
        );
        // A default-surface prefix never errors.
        assert_eq!(
            cache.demoted_layers_under_prefix("wiki").expect("query"),
            None
        );
        // An empty prefix (matches everything, incl. default) never errors.
        assert_eq!(cache.demoted_layers_under_prefix("").expect("query"), None);
    }

    fn tree_slugs(folder: &crate::vault::ExplorerFolder, out: &mut Vec<String>) {
        for note in &folder.notes {
            out.push(note.slug.clone());
        }
        for child in &folder.folders {
            tree_slugs(child, out);
        }
    }

    #[test]
    fn default_selection_hides_demoted_notes_from_tree_and_summaries() {
        let cache = build_layered_cache();

        // Default surface: the demoted clipping is absent from both surfaces.
        let tree = cache
            .explorer_tree(&LayerSelection::default_surface())
            .expect("tree");
        let mut slugs = Vec::new();
        tree_slugs(&tree, &mut slugs);
        assert!(
            slugs.contains(&"page".to_string()),
            "default tree keeps the wiki page"
        );
        assert!(
            !slugs.contains(&"clip".to_string()),
            "default tree must hide the demoted clipping"
        );

        let summaries = cache
            .note_summaries(&LayerSelection::default_surface())
            .expect("summaries");
        let summary_slugs: Vec<String> = summaries.iter().map(|n| n.slug.clone()).collect();
        assert!(summary_slugs.contains(&"page".to_string()));
        assert!(
            !summary_slugs.contains(&"clip".to_string()),
            "default summaries must hide the demoted clipping"
        );

        let recent = cache
            .recently_modified_notes(10, &LayerSelection::default_surface())
            .expect("recent");
        assert!(recent.iter().all(|n| n.slug != "clip"));
    }

    #[test]
    fn selecting_the_layer_reveals_its_demoted_notes() {
        let cache = build_layered_cache();
        let (selection, warnings) =
            LayerSelection::parse(&["sources".to_string()], &["sources".to_string()]);
        assert!(warnings.is_empty());

        let summaries = cache.note_summaries(&selection).expect("summaries");
        let summary_slugs: Vec<String> = summaries.iter().map(|n| n.slug.clone()).collect();
        assert_eq!(
            summary_slugs,
            vec!["clip".to_string()],
            "selecting sources returns only the demoted clipping"
        );

        let recent = cache
            .recently_modified_notes(10, &selection)
            .expect("recent");
        assert!(recent.iter().any(|n| n.slug == "clip"));
        assert!(recent.iter().all(|n| n.slug != "page"));
    }

    #[test]
    fn orphan_status_is_computed_per_selection() {
        // Lonely (default) has one backlink, from a demoted source. Under the
        // default selection that backlink is hidden, so Lonely reads as an
        // orphan; across all layers it is linked and is not an orphan.
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("wiki")).expect("wiki dir");
        fs::create_dir_all(dir.path().join("sources")).expect("sources dir");
        fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");
        fs::write(
            dir.path().join("wiki/Lonely.md"),
            "# Lonely\n\nno outgoing links",
        )
        .expect("lonely");
        fs::write(
            dir.path().join("sources/Src.md"),
            "# Src\n\nrefers to [[Lonely]]",
        )
        .expect("src");

        let cache = SqliteCache::in_memory(384).expect("cache");
        let embedder = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build index");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("populate");

        let default_stats = cache
            .vault_stats(&LayerSelection::default_surface())
            .expect("default stats");
        let default_orphans: Vec<&str> = default_stats
            .orphan_notes
            .iter()
            .map(|n| n.slug.as_str())
            .collect();
        assert!(
            default_orphans.contains(&"lonely"),
            "under default, Lonely's only backlink is from a hidden demoted source, so it is an orphan"
        );

        let all_stats = cache
            .vault_stats(&LayerSelection::all())
            .expect("all stats");
        let all_orphans: Vec<&str> = all_stats
            .orphan_notes
            .iter()
            .map(|n| n.slug.as_str())
            .collect();
        assert!(
            !all_orphans.contains(&"lonely"),
            "across all layers Lonely is linked from Src and is not an orphan"
        );
    }

    #[test]
    fn read_note_exposes_normalized_frontmatter_metadata() {
        let dir = tempdir().expect("temp dir");
        fs::write(
            dir.path().join("Device.md"),
            "---\ntags: [Type/Device, action/review]\naliases: [Router, Gateway]\nstatus: active\nreview-date: 2026-08-01\n---\n# Device\n\n#area/network",
        )
        .expect("write note");
        let cache = SqliteCache::in_memory(384).expect("sqlite cache");
        let embedder = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build index");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("populate cache");

        let note = cache
            .read_note_by_slug("device")
            .expect("read note")
            .expect("device note");
        assert_eq!(
            note.metadata.tags,
            vec!["action/review", "area/network", "type/device"]
        );
        assert_eq!(note.metadata.aliases, vec!["Router", "Gateway"]);
        assert_eq!(
            note.metadata.properties,
            serde_json::json!({"status":"active", "review-date":"2026-08-01"})
        );
    }

    #[test]
    fn recently_modified_notes_returns_newest_source_files_first() {
        let dir = tempdir().expect("temp dir");
        fs::write(dir.path().join("Alpha.md"), "alpha").expect("write alpha");
        fs::write(dir.path().join("Bravo.md"), "bravo").expect("write bravo");
        fs::write(dir.path().join("Charlie.md"), "charlie").expect("write charlie");

        let cache = SqliteCache::in_memory(384).expect("sqlite cache");
        let embedder = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build index");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("populate cache");
        {
            let conn = cache.connection().expect("connection");
            conn.execute(
                "UPDATE notes SET mtime_ns = ?1 WHERE slug = ?2",
                params![10_i64, "alpha"],
            )
            .expect("set alpha mtime");
            conn.execute(
                "UPDATE notes SET mtime_ns = ?1 WHERE slug = ?2",
                params![30_i64, "bravo"],
            )
            .expect("set bravo mtime");
            conn.execute(
                "UPDATE notes SET mtime_ns = ?1 WHERE slug = ?2",
                params![20_i64, "charlie"],
            )
            .expect("set charlie mtime");
        }

        let notes = cache
            .recently_modified_notes(2, &LayerSelection::default_surface())
            .expect("recently modified notes");

        assert_eq!(
            notes
                .iter()
                .map(|note| note.slug.as_str())
                .collect::<Vec<_>>(),
            vec!["bravo", "charlie"]
        );
    }
}
