//! Full-text (FTS5) and vector search queries over the cache.

use std::collections::HashSet;

use rusqlite::params;

use crate::cache::SqliteCache;
use crate::cache::parse::{build_fts_query, fts_query_terms};
use crate::embed::Embedder;
use crate::vault::{SearchHit, content_snippet, normalize_title};

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
#[allow(dead_code)]
pub struct SemanticHit {
    pub chunk_id: i64,
    pub note_slug: String,
    pub heading_path: Option<String>,
    pub content: String,
    pub distance: f32,
}

impl SqliteCache {
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
        let conn = self.read()?;
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

    fn search_title_or_path(
        &self,
        normalized_query: &str,
        limit: usize,
        seen: &mut HashSet<String>,
        results: &mut Vec<SearchHit>,
    ) -> Result<(), String> {
        let like = format!("%{}%", escape_like(normalized_query));
        let conn = self.read()?;
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

        let conn = self.read()?;
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
        let conn = self.read()?;
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
        let conn = self.read()?;
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
