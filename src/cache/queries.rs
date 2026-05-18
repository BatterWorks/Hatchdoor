use std::collections::{BTreeMap, HashSet};

use rusqlite::{OptionalExtension, params};

use crate::embed::Embedder;
use crate::vault::{
    ExplorerFolder, ExplorerNote, ModifiedNote, Note, NoteLink, NoteLinks, SearchHit,
    content_snippet, normalize_link_target, normalize_title, slugify,
};

use super::SqliteCache;
use super::parse::{build_fts_query, fts_query_terms};

impl SqliteCache {
    pub fn read_note_by_slug(&self, slug: &str) -> Result<Option<Note>, String> {
        let conn = self.connection()?;
        conn.query_row(
            r#"
            SELECT title, slug, relative_path, content, content_hash
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
                    content_hash: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("failed to read note '{slug}' from SQLite cache: {error}"))
    }

    pub fn explorer_tree(&self) -> Result<ExplorerFolder, String> {
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

    pub fn recently_modified_notes(
        &self,
        limit: usize,
    ) -> Result<Vec<ModifiedNote>, String> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT title, slug, relative_path, mtime_ns
                FROM notes
                ORDER BY mtime_ns DESC, relative_path ASC
                LIMIT ?1
                "#,
            )
            .map_err(|error| format!("failed to prepare recently modified query: {error}"))?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(ModifiedNote {
                    title: row.get(0)?,
                    slug: row.get(1)?,
                    relative_path: row.get(2)?,
                    mtime_ns: row.get(3)?,
                })
            })
            .map_err(|error| format!("failed to query recently modified notes: {error}"))?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("failed to read recently modified notes: {error}"))
    }

    pub fn search(
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

        self.search_title_or_path(&normalized_query, limit, &mut seen, &mut results)?;
        if !include_content || results.len() >= limit {
            return Ok(results);
        }

        let Some(fts_query) = build_fts_query(query) else {
            return Ok(results);
        };
        let snippet_terms = fts_query_terms(query)
            .into_iter()
            .map(|term| normalize_title(&term))
            .collect::<Vec<_>>();

        let remaining = limit.saturating_sub(results.len());
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
            .query_map(
                params![fts_query, (remaining * 3).max(remaining) as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .map_err(|error| format!("failed to execute SQLite FTS search: {error}"))?;

        for row in rows {
            let (title, slug, relative_path, content) =
                row.map_err(|error| format!("failed to read SQLite FTS result: {error}"))?;
            if seen.contains(&slug) {
                continue;
            }
            seen.insert(slug.clone());
            let snippet = content_snippet(&content, &normalized_query).or_else(|| {
                snippet_terms
                    .iter()
                    .find_map(|term| content_snippet(&content, term))
            });
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

    pub fn note_links(&self, slug: &str) -> Result<Option<NoteLinks>, String> {
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

    pub fn resolve_wikilink(&self, raw_target: &str) -> Result<Option<String>, String> {
        let normalized_target = normalize_link_target(raw_target);
        let normalized_path = normalize_title(&normalized_target);
        let conn = self.connection()?;

        let by_path = conn
            .query_row(
                r#"
                SELECT slug
                FROM notes
                WHERE normalized_relative_path = ?1
                ORDER BY relative_path
                LIMIT 1
                "#,
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
                r#"
                SELECT slug
                FROM notes
                WHERE normalized_title = ?1
                ORDER BY relative_path
                LIMIT 1
                "#,
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

    fn search_title_or_path(
        &self,
        normalized_query: &str,
        limit: usize,
        seen: &mut HashSet<String>,
        results: &mut Vec<SearchHit>,
    ) -> Result<(), String> {
        let like = format!("%{}%", escape_like(normalized_query));
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT title,
                       slug,
                       relative_path,
                       CASE
                         WHEN normalized_title LIKE ?1 ESCAPE '\' THEN 'title'
                         ELSE 'path'
                       END AS match_kind
                FROM notes
                WHERE normalized_title LIKE ?1 ESCAPE '\'
                   OR normalized_relative_path LIKE ?1 ESCAPE '\'
                ORDER BY
                  CASE WHEN normalized_title LIKE ?1 ESCAPE '\' THEN 0 ELSE 1 END,
                  relative_path
                LIMIT ?2
                "#,
            )
            .map_err(|error| format!("failed to prepare title/path search: {error}"))?;
        let rows = stmt
            .query_map(params![like, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| format!("failed to execute title/path search: {error}"))?;

        for row in rows {
            let (title, slug, relative_path, match_kind) =
                row.map_err(|error| format!("failed to read title/path search result: {error}"))?;
            if !seen.insert(slug.clone()) {
                continue;
            }
            results.push(SearchHit {
                title,
                slug,
                relative_path,
                match_kind,
                snippet: None,
            });
        }

        Ok(())
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

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SemanticHit {
    pub chunk_id: i64,
    pub note_slug: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub distance: f32,
}

impl SqliteCache {
    #[allow(dead_code)]
    pub fn semantic_search(
        &self,
        embedder: &dyn Embedder,
        query: &str,
        k: usize,
    ) -> Result<Vec<SemanticHit>, String> {
        let query_vec = embedder
            .embed(&[query.to_string()])?
            .into_iter()
            .next()
            .ok_or("embedder returned no vectors")?;
        let query_bytes: &[u8] = bytemuck::cast_slice(&query_vec);

        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                r#"
            SELECT v.chunk_id, c.note_slug, c.heading_path, c.content, v.distance
            FROM chunk_vectors v
            JOIN chunks c ON c.id = v.chunk_id
            WHERE v.embedding MATCH ?1
              AND v.k = ?2
            ORDER BY v.distance
            "#,
            )
            .map_err(|e| format!("prepare semantic_search: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params![query_bytes, k as i64], |row| {
                Ok(SemanticHit {
                    chunk_id: row.get(0)?,
                    note_slug: row.get(1)?,
                    heading_path: row.get(2)?,
                    content: row.get(3)?,
                    distance: row.get::<_, f64>(4)? as f32,
                })
            })
            .map_err(|e| format!("query semantic_search: {e}"))?;
        let mut hits = Vec::new();
        for row in rows {
            hits.push(row.map_err(|e| format!("read semantic_search row: {e}"))?);
        }
        Ok(hits)
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

fn escape_like(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
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

    #[test]
    fn content_search_snippet_falls_back_to_matched_query_token() {
        let dir = tempdir().expect("temp dir");
        fs::write(dir.path().join("Home.md"), "alpha context only").expect("write note");
        let cache = SqliteCache::in_memory().expect("sqlite cache");
        let embedder = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build index");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("populate cache");

        let hits = cache
            .search("alpha missing", true, 10)
            .expect("search cache");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].match_kind, "content");
        assert_eq!(hits[0].snippet.as_deref(), Some("alpha context only"));
    }

    #[test]
    fn recently_modified_notes_returns_newest_source_files_first() {
        let dir = tempdir().expect("temp dir");
        fs::write(dir.path().join("Alpha.md"), "alpha").expect("write alpha");
        fs::write(dir.path().join("Bravo.md"), "bravo").expect("write bravo");
        fs::write(dir.path().join("Charlie.md"), "charlie").expect("write charlie");

        let cache = SqliteCache::in_memory().expect("sqlite cache");
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
            .recently_modified_notes(2)
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

#[cfg(test)]
mod semantic_search_tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::vault::VaultIndex;

    fn vault_with(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        dir
    }

    #[test]
    fn semantic_search_returns_hits_ordered_by_distance() {
        let dir = vault_with(&[
            ("a.md", "# Apples\n\napples and oranges"),
            ("b.md", "# Bicycles\n\nspokes and wheels"),
        ]);
        let cache = SqliteCache::in_memory().expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");

        let hits = cache
            .semantic_search(embedder.as_ref(), "apples and oranges", 5)
            .expect("search");
        assert!(!hits.is_empty());
        for w in hits.windows(2) {
            assert!(w[0].distance <= w[1].distance);
        }
    }

    #[test]
    fn semantic_search_respects_limit() {
        let dir = vault_with(&[
            ("a.md", "# A\n\nfirst"),
            ("b.md", "# B\n\nsecond"),
            ("c.md", "# C\n\nthird"),
        ]);
        let cache = SqliteCache::in_memory().expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");

        let hits = cache
            .semantic_search(embedder.as_ref(), "anything", 2)
            .expect("search");
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn semantic_search_returns_empty_when_no_chunks() {
        let cache = SqliteCache::in_memory().expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let hits = cache
            .semantic_search(embedder.as_ref(), "anything", 5)
            .expect("search");
        assert!(hits.is_empty());
    }
}
