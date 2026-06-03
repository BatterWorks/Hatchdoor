use std::collections::{BTreeMap, HashMap, HashSet};

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

    pub fn recently_modified_notes(&self, limit: usize) -> Result<Vec<ModifiedNote>, String> {
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

    pub fn resolve_wikilink(&self, raw_target: &str) -> Result<Option<(String, String)>, String> {
        // Strip heading (#) and block (^) anchors — they point within a note, not to a different note
        let note_target = raw_target
            .split('#')
            .next()
            .unwrap_or(raw_target)
            .split('^')
            .next()
            .unwrap_or(raw_target);
        let normalized_target = normalize_link_target(note_target);
        let normalized_path = normalize_title(&normalized_target);
        let conn = self.connection()?;

        let by_path = conn
            .query_row(
                r#"
                SELECT slug, relative_path
                FROM notes
                WHERE normalized_relative_path = ?1
                ORDER BY relative_path
                LIMIT 1
                "#,
                params![normalized_path],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
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
                SELECT slug, relative_path
                FROM notes
                WHERE normalized_title = ?1
                ORDER BY relative_path
                LIMIT 1
                "#,
                params![normalized_base],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| format!("failed to resolve wikilink by title: {error}"))?;
        if by_title.is_some() {
            return Ok(by_title);
        }

        let slug = slugify(base);
        conn.query_row(
            "SELECT slug, relative_path FROM notes WHERE slug = ?1 LIMIT 1",
            params![slug],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
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
pub struct ChunkFtsHit {
    pub chunk_id: i64,
    pub note_slug: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub bm25: f32,
}

#[derive(Debug, Clone)]
pub struct OutboundLinkRow {
    pub slug: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct NoteWithLinks {
    pub slug: String,
    pub title: String,
    pub relative_path: String,
    pub outbound_links: Vec<OutboundLinkRow>,
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
        let prefixed_query = format!("{}{}", embedder.query_prefix(), query);
        let query_vec = embedder
            .embed(&[prefixed_query])?
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

    /// Note-level FTS5 lookup ordered by BM25. Returns slugs in rank order.
    /// Used by the hybrid-retrieval eval. Returns an empty list if the query
    /// produces no usable FTS tokens.
    #[allow(dead_code)]
    pub fn fts_search_notes(&self, query: &str, k: usize) -> Result<Vec<String>, String> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let Some(fts_q) = crate::cache::parse::build_fts_query(query) else {
            return Ok(Vec::new());
        };
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT slug
                FROM note_fts
                WHERE note_fts MATCH ?1
                ORDER BY bm25(note_fts)
                LIMIT ?2
                "#,
            )
            .map_err(|e| format!("prepare fts_search_notes: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params![fts_q, k as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| format!("query fts_search_notes: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("read fts_search_notes row: {e}"))?);
        }
        Ok(out)
    }

    /// Chunk-level FTS5 lookup ordered by BM25. Returns `ChunkFtsHit` rows in
    /// rank order (bm25 ascending, i.e. best match first).
    /// Returns an empty list if the query produces no usable FTS tokens.
    #[allow(dead_code)]
    pub fn fts_search_chunks(&self, query: &str, k: usize) -> Result<Vec<ChunkFtsHit>, String> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let Some(fts_q) = build_fts_query(query) else {
            return Ok(Vec::new());
        };
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT c.id, c.note_slug, c.heading_path, c.content, bm25(chunk_fts)
                FROM chunk_fts
                JOIN chunks c ON c.id = chunk_fts.rowid
                WHERE chunk_fts MATCH ?1
                ORDER BY bm25(chunk_fts)
                LIMIT ?2
                "#,
            )
            .map_err(|e| format!("prepare fts_search_chunks: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params![fts_q, k as i64], |row| {
                Ok(ChunkFtsHit {
                    chunk_id: row.get(0)?,
                    note_slug: row.get(1)?,
                    heading_path: row.get(2)?,
                    content: row.get(3)?,
                    bm25: row.get::<_, f64>(4)? as f32,
                })
            })
            .map_err(|e| format!("query fts_search_chunks: {e}"))?;
        let mut hits = Vec::new();
        for row in rows {
            hits.push(row.map_err(|e| format!("read fts_search_chunks row: {e}"))?);
        }
        Ok(hits)
    }

    pub fn notes_with_outbound_links_batch(
        &self,
        slugs: &[String],
    ) -> Result<HashMap<String, NoteWithLinks>, String> {
        if slugs.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.connection()?;

        let placeholders = std::iter::repeat_n("?", slugs.len())
            .collect::<Vec<_>>()
            .join(",");

        let mut map: HashMap<String, NoteWithLinks> = HashMap::new();

        // Note metadata
        let sql_a =
            format!("SELECT slug, title, relative_path FROM notes WHERE slug IN ({placeholders})");
        let mut stmt_a = conn
            .prepare(&sql_a)
            .map_err(|e| format!("prepare notes batch: {e}"))?;
        let rows_a = stmt_a
            .query_map(rusqlite::params_from_iter(slugs.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("query notes batch: {e}"))?;
        for row in rows_a {
            let (slug, title, relative_path) =
                row.map_err(|e| format!("read notes batch row: {e}"))?;
            map.insert(
                slug.clone(),
                NoteWithLinks {
                    slug,
                    title,
                    relative_path,
                    outbound_links: Vec::new(),
                },
            );
        }

        // Outbound links (only resolved targets — JOIN drops danglers)
        let sql_b = format!(
            "SELECT l.source_slug, t.slug, t.title \
             FROM note_links l \
             JOIN notes t ON t.slug = l.target_slug \
             WHERE l.source_slug IN ({placeholders}) \
             ORDER BY l.source_slug, t.relative_path"
        );
        let mut stmt_b = conn
            .prepare(&sql_b)
            .map_err(|e| format!("prepare links batch: {e}"))?;
        let rows_b = stmt_b
            .query_map(rusqlite::params_from_iter(slugs.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("query links batch: {e}"))?;
        for row in rows_b {
            let (source_slug, target_slug, target_title) =
                row.map_err(|e| format!("read links batch row: {e}"))?;
            if let Some(entry) = map.get_mut(&source_slug) {
                entry.outbound_links.push(OutboundLinkRow {
                    slug: target_slug,
                    title: target_title,
                });
            }
        }

        Ok(map)
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

impl SqliteCache {
    pub fn vault_stats(&self) -> Result<crate::api_types::VaultStatsResponse, String> {
        use crate::api_types::{
            FolderStat, LinkedNoteRef, MonthActivity, NoteList, NoteRef, NoteWordRef, TagStat,
            VaultStatsResponse,
        };

        let conn = self.connection()?;

        let note_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))
            .map_err(|e| format!("vault_stats note_count: {e}"))?;

        let tag_count: i64 = conn
            .query_row("SELECT COUNT(DISTINCT tag) FROM tags", [], |row| row.get(0))
            .map_err(|e| format!("vault_stats tag_count: {e}"))?;

        let link_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM note_links", [], |row| row.get(0))
            .map_err(|e| format!("vault_stats link_count: {e}"))?;

        let vault_size_bytes: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(size_bytes), 0) FROM notes",
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
            .prepare("SELECT slug, title, content FROM notes ORDER BY relative_path")
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
            .prepare(
                "SELECT tag, COUNT(*) as note_count FROM tags GROUP BY tag \
                 ORDER BY note_count DESC, tag LIMIT 20",
            )
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
            .prepare(
                r#"
                SELECT n.title, n.slug, COUNT(l.source_slug) as backlink_count
                FROM notes n
                LEFT JOIN note_links l ON l.target_slug = n.slug
                GROUP BY n.slug
                HAVING backlink_count > 0
                ORDER BY backlink_count DESC, n.title
                LIMIT 20
                "#,
            )
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
            .prepare(
                r#"
                SELECT strftime('%Y-%m', mtime_ns / 1000000000, 'unixepoch') as month,
                       COUNT(*) as modified_count
                FROM notes
                GROUP BY month
                ORDER BY month DESC
                LIMIT 6
                "#,
            )
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
            .prepare(
                r#"
                SELECT
                  CASE WHEN instr(relative_path, '/') > 0
                    THEN substr(relative_path, 1, instr(relative_path, '/') - 1)
                    ELSE ''
                  END as folder,
                  COUNT(*) as note_count
                FROM notes
                GROUP BY folder
                ORDER BY note_count DESC, folder
                "#,
            )
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

        let mut orphan_stmt = conn
            .prepare(
                r#"
                SELECT title, slug FROM notes
                WHERE slug NOT IN (SELECT DISTINCT source_slug FROM note_links)
                  AND slug NOT IN (SELECT DISTINCT target_slug FROM note_links)
                ORDER BY title
                "#,
            )
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
            .prepare(
                r#"
                SELECT title, slug FROM notes
                WHERE slug NOT IN (SELECT DISTINCT note_slug FROM tags)
                ORDER BY title
                "#,
            )
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
                "SELECT COUNT(*) FROM notes \
                 WHERE mtime_ns >= (unixepoch('now') - 7 * 86400) * 1000000000",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("vault_stats week_count: {e}"))?;
        let mut week_stmt = conn
            .prepare(
                r#"
                SELECT title, slug FROM notes
                WHERE mtime_ns >= (unixepoch('now') - 7 * 86400) * 1000000000
                ORDER BY mtime_ns DESC
                LIMIT 20
                "#,
            )
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
                "SELECT COUNT(*) FROM notes \
                 WHERE mtime_ns >= (unixepoch('now') - 30 * 86400) * 1000000000",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("vault_stats month_count: {e}"))?;
        let mut month_stmt = conn
            .prepare(
                r#"
                SELECT title, slug FROM notes
                WHERE mtime_ns >= (unixepoch('now') - 30 * 86400) * 1000000000
                ORDER BY mtime_ns DESC
                LIMIT 20
                "#,
            )
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

    pub fn graph_data(&self) -> Result<crate::api_types::GraphResponse, String> {
        use crate::api_types::{GraphEdge, GraphNode, GraphResponse};

        let conn = self.connection()?;

        let mut nodes_stmt = conn
            .prepare(
                r#"
                SELECT n.slug, n.title,
                  (SELECT tag FROM tags WHERE note_slug = n.slug ORDER BY tag LIMIT 1) as primary_tag,
                  COUNT(l.source_slug) as backlink_count
                FROM notes n
                LEFT JOIN note_links l ON l.target_slug = n.slug
                GROUP BY n.slug
                ORDER BY n.title
                "#,
            )
            .map_err(|e| format!("graph_data prepare nodes: {e}"))?;
        let nodes: Vec<GraphNode> = nodes_stmt
            .query_map([], |row| {
                Ok(GraphNode {
                    slug: row.get(0)?,
                    title: row.get(1)?,
                    primary_tag: row.get(2)?,
                    backlink_count: row.get(3)?,
                })
            })
            .map_err(|e| format!("graph_data query nodes: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("graph_data read nodes: {e}"))?;
        drop(nodes_stmt);

        let mut edges_stmt = conn
            .prepare("SELECT source_slug, target_slug FROM note_links")
            .map_err(|e| format!("graph_data prepare edges: {e}"))?;
        let edges: Vec<GraphEdge> = edges_stmt
            .query_map([], |row| {
                Ok(GraphEdge {
                    source: row.get(0)?,
                    target: row.get(1)?,
                })
            })
            .map_err(|e| format!("graph_data query edges: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("graph_data read edges: {e}"))?;
        drop(edges_stmt);

        Ok(GraphResponse { nodes, edges })
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
        let cache = SqliteCache::in_memory(384).expect("sqlite cache");
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
        let cache = SqliteCache::in_memory(384).expect("open");
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
        let cache = SqliteCache::in_memory(384).expect("open");
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
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let hits = cache
            .semantic_search(embedder.as_ref(), "anything", 5)
            .expect("search");
        assert!(hits.is_empty());
    }
}

#[cfg(test)]
mod fts_search_chunks_tests {
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

    fn build_cache(files: &[(&str, &str)]) -> SqliteCache {
        let dir = vault_with(files);
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");
        cache
    }

    #[test]
    fn fts_search_chunks_returns_hits_ordered_by_bm25() {
        let cache = build_cache(&[
            ("a.md", "# Apples\n\napples and oranges grow on trees"),
            ("b.md", "# Bicycles\n\nspokes and wheels"),
        ]);
        let hits = cache.fts_search_chunks("apples", 10).expect("search");
        assert!(!hits.is_empty(), "expected at least one hit");
        assert!(hits[0].note_slug.contains('a') || hits[0].content.contains("apples"));
        for w in hits.windows(2) {
            assert!(w[0].bm25 <= w[1].bm25, "bm25 must be non-decreasing");
        }
    }

    #[test]
    fn fts_search_chunks_returns_empty_on_stopword_only_query() {
        let cache = build_cache(&[("a.md", "# A\n\nbody text")]);
        let hits = cache.fts_search_chunks("   .  ", 10).expect("search");
        assert!(hits.is_empty());
    }

    #[test]
    fn fts_search_chunks_respects_limit() {
        let cache = build_cache(&[
            ("a.md", "# A\n\napples"),
            ("b.md", "# B\n\napples"),
            ("c.md", "# C\n\napples"),
        ]);
        let hits = cache.fts_search_chunks("apples", 2).expect("search");
        assert_eq!(hits.len(), 2);
    }
}

#[cfg(test)]
mod notes_with_outbound_links_batch_tests {
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

    fn build_cache(files: &[(&str, &str)]) -> SqliteCache {
        let dir = vault_with(files);
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");
        cache
    }

    #[test]
    fn batch_returns_note_metadata_for_each_slug() {
        let cache = build_cache(&[
            ("Alpha.md", "# Alpha\n\nbody"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let map = cache
            .notes_with_outbound_links_batch(&["alpha".to_string(), "bravo".to_string()])
            .expect("batch");
        assert_eq!(map.len(), 2);
        let a = map.get("alpha").expect("alpha");
        assert_eq!(a.title, "Alpha");
        assert_eq!(a.relative_path, "Alpha");
        assert!(a.outbound_links.is_empty());
    }

    #[test]
    fn batch_returns_resolved_outbound_links_only() {
        let cache = build_cache(&[
            ("Alpha.md", "# Alpha\n\nlinks to [[Bravo]] and [[Ghost]]"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let map = cache
            .notes_with_outbound_links_batch(&["alpha".to_string()])
            .expect("batch");
        let a = map.get("alpha").expect("alpha");
        assert_eq!(a.outbound_links.len(), 1);
        assert_eq!(a.outbound_links[0].slug, "bravo");
        assert_eq!(a.outbound_links[0].title, "Bravo");
    }

    #[test]
    fn batch_omits_missing_slugs() {
        let cache = build_cache(&[("Alpha.md", "# Alpha\n\nbody")]);
        let map = cache
            .notes_with_outbound_links_batch(&["alpha".to_string(), "ghost".to_string()])
            .expect("batch");
        assert!(map.contains_key("alpha"));
        assert!(!map.contains_key("ghost"));
    }

    #[test]
    fn batch_empty_input_returns_empty_map() {
        let cache = build_cache(&[("Alpha.md", "# Alpha\n\nbody")]);
        let map = cache.notes_with_outbound_links_batch(&[]).expect("batch");
        assert!(map.is_empty());
    }
}
