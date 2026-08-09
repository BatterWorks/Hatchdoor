//! Full-text (FTS5) and vector search queries over the cache.

use std::collections::HashSet;

use rusqlite::{Connection, params};

use crate::cache::SqliteCache;
use crate::cache::parse::{build_fts_query, fts_query_terms};
use crate::embed::Embedder;
use crate::search::LayerSelection;
use crate::vault::{SearchHit, content_snippet, normalize_title};
use crate::vault_registry::VaultId;

/// The default-surface semantic KNN query (against `chunk_vectors`). Kept as a
/// named constant so a test can assert the default search path runs THIS query —
/// the unfiltered vec0 KNN — rather than the Rust full-scan fallback.
pub(crate) const DEFAULT_SEMANTIC_KNN_SQL: &str = r#"
    SELECT v.chunk_id, c.note_slug, c.heading_path, c.content, v.distance
    FROM chunk_vectors v
    JOIN chunks c ON c.id = v.chunk_id
    WHERE v.embedding MATCH ?1
      AND v.k = ?2
    ORDER BY v.distance
    "#;

/// The per-layer demoted KNN query. `?3` binds the layer, which is the vec0
/// PARTITION KEY, so the scan is pruned to that partition and stays a KNN.
const DEMOTED_SEMANTIC_KNN_SQL: &str = r#"
    SELECT v.chunk_id, c.note_slug, c.heading_path, c.content, v.distance
    FROM chunk_vectors_demoted v
    JOIN chunks c ON c.id = v.chunk_id
    WHERE v.embedding MATCH ?1
      AND v.k = ?2
      AND v.layer = ?3
    ORDER BY v.distance
    "#;

/// The demoted KNN query across every layer (for the `all` selector).
const DEMOTED_SEMANTIC_KNN_ALL_SQL: &str = r#"
    SELECT v.chunk_id, c.note_slug, c.heading_path, c.content, v.distance
    FROM chunk_vectors_demoted v
    JOIN chunks c ON c.id = v.chunk_id
    WHERE v.embedding MATCH ?1
      AND v.k = ?2
    ORDER BY v.distance
    "#;

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

#[derive(Debug, Clone)]
pub(crate) struct VaultSemanticHit {
    pub(crate) vault_id: VaultId,
    pub(crate) chunk_id: i64,
    pub(crate) note_slug: String,
    pub(crate) heading_path: Option<String>,
    pub(crate) content: String,
    pub(crate) distance: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct VaultChunkFtsHit {
    pub(crate) vault_id: VaultId,
    pub(crate) chunk_id: i64,
    pub(crate) note_slug: String,
    pub(crate) heading_path: Option<String>,
    pub(crate) content: String,
    pub(crate) bm25: f32,
}

impl SqliteCache {
    /// Vault-qualified variant of the established layered KNN path. Scope is
    /// part of the SQL eligibility predicate before sqlite-vec ranks chunks,
    /// so an all-Vault request has one global result window rather than a
    /// merged set of per-Vault windows.
    pub(crate) fn vault_semantic_search_layered(
        &self,
        conn: &Connection,
        vault_ids: &[VaultId],
        embedder: &dyn Embedder,
        query: &str,
        k: usize,
        selection: &LayerSelection,
    ) -> Result<Vec<VaultSemanticHit>, String> {
        if vault_ids.is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        let prefixed_query = format!("{}{}", embedder.query_prefix(), query);
        let query_vec = embedder
            .embed(&[prefixed_query])?
            .into_iter()
            .next()
            .ok_or("embedder returned no vectors")?;
        let query_bytes: &[u8] = bytemuck::cast_slice(&query_vec);
        let ids = vault_ids
            .iter()
            .map(|vault_id| format!("'{}'", vault_id))
            .collect::<Vec<_>>()
            .join(", ");
        let mut hits = Vec::new();
        let mut collect = |sql: String| -> Result<(), String> {
            let mut statement = conn
                .prepare(&sql)
                .map_err(|error| format!("prepare Vault semantic KNN: {error}"))?;
            let rows = statement
                .query_map(params![query_bytes, k as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, f64>(5)? as f32,
                    ))
                })
                .map_err(|error| format!("query Vault semantic KNN: {error}"))?;
            for row in rows {
                let (vault_id, chunk_id, note_slug, heading_path, content, distance) =
                    row.map_err(|error| format!("read Vault semantic KNN row: {error}"))?;
                hits.push(VaultSemanticHit {
                    vault_id: vault_id
                        .parse()
                        .map_err(|error| format!("read Vault semantic identity: {error}"))?,
                    chunk_id,
                    note_slug,
                    heading_path,
                    content,
                    distance,
                });
            }
            Ok(())
        };

        if selection.includes_default() {
            collect(format!(
                "SELECT c.vault_id, v.chunk_id, c.note_slug, c.heading_path, c.content, v.distance \
                 FROM vault_chunk_vectors v JOIN vault_chunks c ON c.id = v.chunk_id \
                 WHERE v.embedding MATCH ?1 AND v.k = ?2 AND v.vault_id IN ({ids}) \
                 ORDER BY v.distance"
            ))?;
        }
        if selection.is_all() {
            collect(format!(
                "SELECT c.vault_id, v.chunk_id, c.note_slug, c.heading_path, c.content, v.distance \
                 FROM vault_chunk_vectors_demoted v JOIN vault_chunks c ON c.id = v.chunk_id \
                 WHERE v.embedding MATCH ?1 AND v.k = ?2 AND v.vault_id IN ({ids}) \
                 ORDER BY v.distance"
            ))?;
        } else {
            for layer in selection.named_layers() {
                collect(format!(
                    "SELECT c.vault_id, v.chunk_id, c.note_slug, c.heading_path, c.content, v.distance \
                     FROM vault_chunk_vectors_demoted v JOIN vault_chunks c ON c.id = v.chunk_id \
                     WHERE v.embedding MATCH ?1 AND v.k = ?2 AND v.layer = '{}' \
                       AND v.vault_id IN ({ids}) ORDER BY v.distance",
                    layer.replace('\'', "''"),
                ))?;
            }
        }
        hits.sort_by(|left, right| {
            left.distance
                .total_cmp(&right.distance)
                .then_with(|| left.vault_id.cmp(&right.vault_id))
                .then_with(|| left.note_slug.cmp(&right.note_slug))
                .then_with(|| left.chunk_id.cmp(&right.chunk_id))
        });
        hits.truncate(k);
        Ok(hits)
    }

    /// Globally rank keyword hits across the given already-participating Vault
    /// snapshots. The caller owns snapshot status; this method keeps the FTS
    /// BM25 window global rather than merging per-Vault result windows.
    pub(crate) fn vault_fts_search_chunks(
        &self,
        conn: &Connection,
        vault_ids: &[VaultId],
        query: &str,
        selection: &LayerSelection,
    ) -> Result<Vec<VaultChunkFtsHit>, String> {
        if vault_ids.is_empty() {
            return Ok(Vec::new());
        }
        let Some(fts_q) = build_fts_query(query) else {
            return Ok(Vec::new());
        };
        let ids = vault_ids
            .iter()
            .map(|vault_id| format!("'{}'", vault_id))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            r#"
            SELECT c.vault_id, c.id, c.note_slug, c.heading_path, c.content, bm25(vault_chunk_fts)
            FROM vault_chunk_fts
            JOIN vault_chunks c ON c.id = vault_chunk_fts.rowid
            JOIN vault_notes n ON n.vault_id = c.vault_id AND n.slug = c.note_slug
            WHERE vault_chunk_fts MATCH ?1
              AND c.vault_id IN ({ids})
              AND {layer}
            ORDER BY bm25(vault_chunk_fts), c.vault_id, c.note_slug, c.id
            "#,
            layer = selection.sql_filter("n.layer"),
        );
        let mut statement = conn
            .prepare(&sql)
            .map_err(|error| format!("prepare Vault FTS search: {error}"))?;
        let rows = statement
            .query_map(params![fts_q], |row| {
                let vault_id = row.get::<_, String>(0)?;
                Ok((
                    vault_id,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, f64>(5)? as f32,
                ))
            })
            .map_err(|error| format!("query Vault FTS search: {error}"))?;
        let mut hits = Vec::new();
        for row in rows {
            let (vault_id, chunk_id, note_slug, heading_path, content, bm25) =
                row.map_err(|error| format!("read Vault FTS search row: {error}"))?;
            hits.push(VaultChunkFtsHit {
                vault_id: vault_id
                    .parse()
                    .map_err(|error| format!("read Vault FTS identity: {error}"))?,
                chunk_id,
                note_slug,
                heading_path,
                content,
                bm25,
            });
        }
        Ok(hits)
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
            .prepare(DEFAULT_SEMANTIC_KNN_SQL)
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

    /// Layer-aware semantic search on the fast vec0 KNN path (no note filters).
    ///
    /// Runs an unfiltered KNN against each vec0 table the selection covers — the
    /// default table (`chunk_vectors`) for the default surface and the demoted
    /// table (`chunk_vectors_demoted`, partition-pruned per layer) for named
    /// layers — then merges by distance. No path scans a table it does not
    /// select, so default search never touches a demoted vector and never falls
    /// onto the Rust full-scan path.
    pub fn semantic_search_layered(
        &self,
        embedder: &dyn Embedder,
        query: &str,
        k: usize,
        selection: &LayerSelection,
    ) -> Result<Vec<SemanticHit>, String> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let prefixed_query = format!("{}{}", embedder.query_prefix(), query);
        let query_vec = embedder
            .embed(&[prefixed_query])?
            .into_iter()
            .next()
            .ok_or("embedder returned no vectors")?;
        let query_bytes: &[u8] = bytemuck::cast_slice(&query_vec);

        let conn = self.read()?;
        let read_hit = |row: &rusqlite::Row<'_>| -> rusqlite::Result<SemanticHit> {
            Ok(SemanticHit {
                chunk_id: row.get(0)?,
                note_slug: row.get(1)?,
                heading_path: row.get(2)?,
                content: row.get(3)?,
                distance: row.get::<_, f64>(4)? as f32,
            })
        };
        let mut hits = Vec::new();

        if selection.includes_default() {
            let mut stmt = conn
                .prepare(DEFAULT_SEMANTIC_KNN_SQL)
                .map_err(|e| format!("prepare default KNN: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params![query_bytes, k as i64], read_hit)
                .map_err(|e| format!("query default KNN: {e}"))?;
            for row in rows {
                hits.push(row.map_err(|e| format!("read default KNN row: {e}"))?);
            }
        }

        if selection.is_all() {
            let mut stmt = conn
                .prepare(DEMOTED_SEMANTIC_KNN_ALL_SQL)
                .map_err(|e| format!("prepare demoted-all KNN: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params![query_bytes, k as i64], read_hit)
                .map_err(|e| format!("query demoted-all KNN: {e}"))?;
            for row in rows {
                hits.push(row.map_err(|e| format!("read demoted-all KNN row: {e}"))?);
            }
        } else {
            for layer in selection.named_layers() {
                let mut stmt = conn
                    .prepare(DEMOTED_SEMANTIC_KNN_SQL)
                    .map_err(|e| format!("prepare demoted KNN: {e}"))?;
                let rows = stmt
                    .query_map(rusqlite::params![query_bytes, k as i64, layer], read_hit)
                    .map_err(|e| format!("query demoted KNN: {e}"))?;
                for row in rows {
                    hits.push(row.map_err(|e| format!("read demoted KNN row: {e}"))?);
                }
            }
        }

        hits.sort_by(|left, right| left.distance.total_cmp(&right.distance));
        hits.truncate(k);
        Ok(hits)
    }

    /// Layer-aware semantic search with a note-slug filter (tags/path/property
    /// filters are present). This is the accepted slow path — it reads every
    /// vector in the SELECTED tables and scores in Rust — but it is only entered
    /// when note filters exist, never for plain layer separation, and it reads
    /// only the tables the selection covers.
    pub fn semantic_search_filtered(
        &self,
        embedder: &dyn Embedder,
        query: &str,
        k: usize,
        selection: &LayerSelection,
        eligible_slugs: &HashSet<String>,
    ) -> Result<Vec<SemanticHit>, String> {
        if k == 0 || eligible_slugs.is_empty() {
            return Ok(Vec::new());
        }
        let prefixed_query = format!("{}{}", embedder.query_prefix(), query);
        let query_vector = embedder
            .embed(&[prefixed_query])?
            .into_iter()
            .next()
            .ok_or("embedder returned no vectors")?;
        let conn = self.read()?;
        let mut hits = Vec::new();

        // Each (sql, params) scan reads chunk_id, note_slug, heading_path,
        // content, embedding for one selected table.
        let mut scan = |sql: &str, layer: Option<&str>| -> Result<(), String> {
            let mut stmt = conn
                .prepare(sql)
                .map_err(|error| format!("prepare filtered semantic scan: {error}"))?;
            let map_row = |row: &rusqlite::Row<'_>| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                ))
            };
            let rows = match layer {
                None => stmt.query_map([], map_row),
                Some(layer) => stmt.query_map(rusqlite::params![layer], map_row),
            }
            .map_err(|error| format!("query filtered semantic scan: {error}"))?;
            for row in rows {
                let (chunk_id, note_slug, heading_path, content, bytes) =
                    row.map_err(|error| format!("read filtered semantic row: {error}"))?;
                if !eligible_slugs.contains(&note_slug) {
                    continue;
                }
                if bytes.len() != query_vector.len() * std::mem::size_of::<f32>() {
                    return Err(format!(
                        "cached embedding dimension mismatch for chunk {chunk_id}"
                    ));
                }
                let distance = bytes
                    .chunks_exact(4)
                    .zip(&query_vector)
                    .map(|(bytes, query_value)| {
                        let value = f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                        let difference = value - query_value;
                        difference * difference
                    })
                    .sum::<f32>()
                    .sqrt();
                hits.push(SemanticHit {
                    chunk_id,
                    note_slug,
                    heading_path,
                    content,
                    distance,
                });
            }
            Ok(())
        };

        if selection.includes_default() {
            scan(
                "SELECT v.chunk_id, c.note_slug, c.heading_path, c.content, v.embedding \
                 FROM chunk_vectors v JOIN chunks c ON c.id = v.chunk_id",
                None,
            )?;
        }
        if selection.is_all() {
            scan(
                "SELECT v.chunk_id, c.note_slug, c.heading_path, c.content, v.embedding \
                 FROM chunk_vectors_demoted v JOIN chunks c ON c.id = v.chunk_id",
                None,
            )?;
        } else {
            for layer in selection.named_layers() {
                scan(
                    "SELECT v.chunk_id, c.note_slug, c.heading_path, c.content, v.embedding \
                     FROM chunk_vectors_demoted v JOIN chunks c ON c.id = v.chunk_id \
                     WHERE v.layer = ?1",
                    Some(&layer),
                )?;
            }
        }

        hits.sort_by(|left, right| left.distance.total_cmp(&right.distance));
        hits.truncate(k);
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

    /// Layer-aware keyword search (no note filters). Applies the same
    /// `LayerSelection` as semantic search by joining `notes` and constraining
    /// `notes.layer` — a cheap, indexed SQL predicate, so keyword search over the
    /// default surface never returns a demoted chunk.
    pub fn fts_search_chunks_layered(
        &self,
        query: &str,
        k: usize,
        selection: &LayerSelection,
    ) -> Result<Vec<ChunkFtsHit>, String> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let Some(fts_q) = build_fts_query(query) else {
            return Ok(Vec::new());
        };
        let conn = self.read()?;
        let sql = format!(
            r#"
            SELECT c.id, c.note_slug, c.heading_path, c.content, bm25(chunk_fts)
            FROM chunk_fts
            JOIN chunks c ON c.id = chunk_fts.rowid
            JOIN notes n ON n.slug = c.note_slug
            WHERE chunk_fts MATCH ?1
              AND {layer}
            ORDER BY bm25(chunk_fts)
            LIMIT ?2
            "#,
            layer = selection.sql_filter("n.layer"),
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare layered FTS search: {e}"))?;
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
            .map_err(|e| format!("query layered FTS search: {e}"))?;
        let mut hits = Vec::new();
        for row in rows {
            hits.push(row.map_err(|e| format!("read layered FTS row: {e}"))?);
        }
        Ok(hits)
    }

    pub fn fts_search_chunks_filtered(
        &self,
        query: &str,
        k: usize,
        selection: &LayerSelection,
        eligible_slugs: &HashSet<String>,
    ) -> Result<Vec<ChunkFtsHit>, String> {
        if k == 0 || eligible_slugs.is_empty() {
            return Ok(Vec::new());
        }
        let Some(fts_q) = build_fts_query(query) else {
            return Ok(Vec::new());
        };
        let conn = self.read()?;
        let sql = format!(
            r#"
                SELECT c.id, c.note_slug, c.heading_path, c.content, bm25(chunk_fts)
                FROM chunk_fts
                JOIN chunks c ON c.id = chunk_fts.rowid
                JOIN notes n ON n.slug = c.note_slug
                WHERE chunk_fts MATCH ?1
                  AND {layer}
                ORDER BY bm25(chunk_fts)
                "#,
            layer = selection.sql_filter("n.layer"),
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|error| format!("prepare filtered FTS search: {error}"))?;
        let rows = stmt
            .query_map(params![fts_q], |row| {
                Ok(ChunkFtsHit {
                    chunk_id: row.get(0)?,
                    note_slug: row.get(1)?,
                    heading_path: row.get(2)?,
                    content: row.get(3)?,
                    bm25: row.get::<_, f64>(4)? as f32,
                })
            })
            .map_err(|error| format!("query filtered FTS search: {error}"))?;
        let mut hits = Vec::new();
        for row in rows {
            let hit = row.map_err(|error| format!("read filtered FTS row: {error}"))?;
            if eligible_slugs.contains(&hit.note_slug) {
                hits.push(hit);
                if hits.len() >= k {
                    break;
                }
            }
        }
        Ok(hits)
    }
}

pub(crate) fn escape_like(input: &str) -> String {
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
mod layer_search_tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::search::LayerSelection;
    use crate::vault::VaultIndex;

    /// A vault with a default-surface note (`wiki/Melatonin.md`) and a demoted
    /// `sources/` note carrying the same distinctive term.
    fn demoted_vault() -> (SqliteCache, Arc<dyn Embedder>) {
        let dir = TempDir::new().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("wiki")).expect("wiki dir");
        std::fs::create_dir_all(dir.path().join("sources")).expect("sources dir");
        std::fs::write(
            dir.path().join("wiki/Compiled.md"),
            "# Compiled\n\nmelatonin regulates the circadian rhythm",
        )
        .expect("write compiled");
        std::fs::write(
            dir.path().join("sources/Clipping.md"),
            "# Clipping\n\nmelatonin regulates the circadian rhythm",
        )
        .expect("write clipping");
        std::fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");

        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");
        (cache, embedder)
    }

    fn slugs(hits: &[super::SemanticHit]) -> Vec<String> {
        hits.iter().map(|h| h.note_slug.clone()).collect()
    }

    #[test]
    fn default_semantic_search_excludes_demoted_but_layer_search_includes_it() {
        let (cache, embedder) = demoted_vault();

        // Default surface: the demoted clipping must NOT appear.
        let default_hits = cache
            .semantic_search_layered(
                embedder.as_ref(),
                "melatonin circadian",
                10,
                &LayerSelection::default_surface(),
            )
            .expect("default search");
        let default_slugs = slugs(&default_hits);
        assert!(
            default_slugs.contains(&"compiled".to_string()),
            "default note present: {default_slugs:?}"
        );
        assert!(
            !default_slugs.contains(&"clipping".to_string()),
            "demoted clipping must be absent from the default surface: {default_slugs:?}"
        );

        // Selecting the `sources` layer surfaces the clipping.
        let (selection, warnings) =
            LayerSelection::parse(&["sources".to_string()], &["sources".to_string()]);
        assert!(warnings.is_empty());
        let layer_hits = cache
            .semantic_search_layered(embedder.as_ref(), "melatonin circadian", 10, &selection)
            .expect("layer search");
        let layer_slugs = slugs(&layer_hits);
        assert!(
            layer_slugs.contains(&"clipping".to_string()),
            "demoted clipping present when its layer is selected: {layer_slugs:?}"
        );
        assert!(
            !layer_slugs.contains(&"compiled".to_string()),
            "a `sources`-only search must not return the default note: {layer_slugs:?}"
        );

        // `all` unions both.
        let all_hits = cache
            .semantic_search_layered(
                embedder.as_ref(),
                "melatonin circadian",
                10,
                &LayerSelection::all(),
            )
            .expect("all search");
        let all_slugs = slugs(&all_hits);
        assert!(all_slugs.contains(&"compiled".to_string()));
        assert!(all_slugs.contains(&"clipping".to_string()));
    }

    #[test]
    fn default_keyword_search_excludes_demoted_but_layer_search_includes_it() {
        let (cache, _embedder) = demoted_vault();

        let default_hits = cache
            .fts_search_chunks_layered("melatonin", 10, &LayerSelection::default_surface())
            .expect("default keyword");
        let default: Vec<String> = default_hits.iter().map(|h| h.note_slug.clone()).collect();
        assert!(default.contains(&"compiled".to_string()), "{default:?}");
        assert!(!default.contains(&"clipping".to_string()), "{default:?}");

        let (selection, _) =
            LayerSelection::parse(&["sources".to_string()], &["sources".to_string()]);
        let layer_hits = cache
            .fts_search_chunks_layered("melatonin", 10, &selection)
            .expect("layer keyword");
        let layer: Vec<String> = layer_hits.iter().map(|h| h.note_slug.clone()).collect();
        assert!(layer.contains(&"clipping".to_string()), "{layer:?}");
        assert!(!layer.contains(&"compiled".to_string()), "{layer:?}");
    }

    /// The default semantic path must be the unfiltered vec0 KNN, never the Rust
    /// full-scan fallback. vec0 encodes its query plan in the idxStr shown by
    /// EXPLAIN QUERY PLAN: `VIRTUAL TABLE INDEX 0:3…` is the KNN plan, `0:1` is a
    /// full table scan. Assert the default query is KNN and that a bare scan of
    /// the same table (what the filtered fallback runs) is the full-scan plan —
    /// proving the two paths are genuinely different query plans.
    #[test]
    fn default_semantic_query_uses_the_vec0_knn_plan_not_a_full_scan() {
        let (cache, _embedder) = demoted_vault();
        let conn = cache.read().expect("read conn");

        let dummy_vec = vec![0.0f32; 384];
        let dummy_bytes: &[u8] = bytemuck::cast_slice(&dummy_vec);
        let knn_plan: String = conn
            .query_row(
                &format!("EXPLAIN QUERY PLAN {}", super::DEFAULT_SEMANTIC_KNN_SQL),
                rusqlite::params![dummy_bytes, 10_i64],
                |row| row.get(3),
            )
            .expect("explain default knn");
        assert!(
            knn_plan.contains("VIRTUAL TABLE INDEX 0:3"),
            "default semantic query must use the vec0 KNN plan (0:3), got: {knn_plan}"
        );

        let scan_plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT v.chunk_id FROM chunk_vectors v JOIN chunks c ON c.id = v.chunk_id",
                [],
                |row| row.get(3),
            )
            .expect("explain full scan");
        assert!(
            scan_plan.contains("VIRTUAL TABLE INDEX 0:1"),
            "the full-scan fallback plan must be a vec0 fullscan (0:1), got: {scan_plan}"
        );
        assert_ne!(
            knn_plan, scan_plan,
            "the fast KNN path and the full-scan path must be different query plans"
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
