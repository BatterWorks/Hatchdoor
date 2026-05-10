use std::collections::{BTreeMap, HashSet};

use rusqlite::{params, OptionalExtension};

use crate::vault::{
    content_snippet, normalize_link_target, normalize_title, slugify, ExplorerFolder, ExplorerNote,
    Note, NoteLink, NoteLinks, SearchHit,
};

use super::parse::build_fts_query;
use super::SqliteCache;

impl SqliteCache {
    pub(crate) fn read_note_by_slug(&self, slug: &str) -> Result<Option<Note>, String> {
        let conn = self.connection()?;
        conn.query_row(
            r#"
            SELECT title, slug, relative_path, content
            FROM notes
            WHERE slug = ?1
            "#,
            params![slug],
            |row| {
                Ok(Note {
                    title: row.get(0)?,
                    slug: row.get(1)?,
                    relative_path: row.get(2)?,
                    content: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("failed to read note '{slug}' from SQLite cache: {error}"))
    }

    pub(crate) fn explorer_tree(&self) -> Result<ExplorerFolder, String> {
        let rows = self.note_rows_ordered()?;
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

    pub(crate) fn search(
        &self,
        query: &str,
        include_content: bool,
        limit: usize,
    ) -> Result<Vec<SearchHit>, String> {
        let normalized_query = normalize_title(query);
        if normalized_query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        let mut seen = HashSet::new();

        for note in self.note_rows_ordered()? {
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
                    title: note.title,
                    slug: note.slug,
                    relative_path: note.relative_path,
                    match_kind: kind.to_string(),
                    snippet: None,
                });
                if results.len() >= limit {
                    return Ok(results);
                }
            }
        }

        if !include_content || results.len() >= limit {
            return Ok(results);
        }

        let Some(fts_query) = build_fts_query(query) else {
            return Ok(results);
        };

        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT title, slug, relative_path, content
                FROM note_fts
                WHERE note_fts MATCH ?1
                ORDER BY bm25(note_fts)
                LIMIT ?2
                "#,
            )
            .map_err(|error| format!("failed to prepare SQLite FTS search: {error}"))?;
        let rows = stmt
            .query_map(params![fts_query, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| format!("failed to execute SQLite FTS search: {error}"))?;

        for row in rows {
            let (title, slug, relative_path, content) =
                row.map_err(|error| format!("failed to read SQLite FTS result: {error}"))?;
            if seen.contains(&slug) {
                continue;
            }
            seen.insert(slug.clone());
            let snippet = content_snippet(&content, &normalized_query);
            results.push(SearchHit {
                title,
                slug,
                relative_path,
                match_kind: "content".to_string(),
                snippet,
            });
            if results.len() >= limit {
                break;
            }
        }

        Ok(results)
    }

    pub(crate) fn note_links(&self, slug: &str) -> Result<Option<NoteLinks>, String> {
        if !self.note_exists(slug)? {
            return Ok(None);
        }

        let outgoing = self.link_rows(
            r#"
            SELECT target.title, target.slug, target.relative_path
            FROM note_links links
            JOIN notes target ON target.slug = links.target_slug
            WHERE links.source_slug = ?1
            ORDER BY target.relative_path
            "#,
            slug,
        )?;
        let backlinks = self.link_rows(
            r#"
            SELECT source.title, source.slug, source.relative_path
            FROM note_links links
            JOIN notes source ON source.slug = links.source_slug
            WHERE links.target_slug = ?1
            ORDER BY source.relative_path
            "#,
            slug,
        )?;

        Ok(Some(NoteLinks {
            outgoing,
            backlinks,
        }))
    }

    pub(crate) fn resolve_wikilink(&self, raw_target: &str) -> Result<Option<String>, String> {
        let normalized_target = normalize_link_target(raw_target);
        let normalized_path = normalize_title(&normalized_target);
        let conn = self.connection()?;

        let by_path = conn
            .query_row(
                "SELECT slug FROM notes WHERE lower(relative_path) = ?1 ORDER BY relative_path LIMIT 1",
                params![normalized_path],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("failed to resolve wikilink by path: {error}"))?;
        if by_path.is_some() {
            return Ok(by_path);
        }

        let base = normalized_target
            .rsplit('/')
            .next()
            .unwrap_or(&normalized_target);
        let normalized_base = normalize_title(base);
        let by_title = conn
            .query_row(
                "SELECT slug FROM notes WHERE lower(title) = ?1 ORDER BY relative_path LIMIT 1",
                params![normalized_base],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("failed to resolve wikilink by title: {error}"))?;
        if by_title.is_some() {
            return Ok(by_title);
        }

        let slug = slugify(base);
        conn.query_row(
            "SELECT slug FROM notes WHERE slug = ?1 LIMIT 1",
            params![slug],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("failed to resolve wikilink by slug: {error}"))
    }

    fn note_rows_ordered(&self) -> Result<Vec<NoteRow>, String> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare("SELECT title, slug, relative_path FROM notes ORDER BY relative_path")
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

    fn note_exists(&self, slug: &str) -> Result<bool, String> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM notes WHERE slug = ?1)",
            params![slug],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("failed checking note existence for '{slug}': {error}"))
    }

    fn link_rows(&self, sql: &str, slug: &str) -> Result<Vec<NoteLink>, String> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(sql)
            .map_err(|error| format!("failed to prepare link query: {error}"))?;
        let rows = stmt
            .query_map(params![slug], |row| {
                Ok(NoteLink {
                    title: row.get(0)?,
                    slug: row.get(1)?,
                    relative_path: row.get(2)?,
                })
            })
            .map_err(|error| format!("failed to query note links: {error}"))?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("failed to read note links: {error}"))
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
