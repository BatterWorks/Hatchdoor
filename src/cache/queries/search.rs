//! Full-text (FTS5) and vector search queries over the cache.

use rusqlite::{Connection, params};

use crate::cache::SqliteCache;
use crate::cache::parse::build_fts_query;
use crate::embed::Embedder;
use crate::search::LayerSelection;
use crate::vault_registry::VaultId;

/// The Vault-qualified default-surface KNN query. Built here rather than
/// inline so the plan-guard test EXPLAINs exactly what production executes: a
/// copy of this string in the test could drift back to a full scan unnoticed,
/// which is the failure the guard exists to catch.
fn default_layer_knn_sql(ids: &str) -> String {
    format!(
        "SELECT c.vault_id, v.chunk_id, c.note_slug, c.heading_path, c.content, v.distance \
         FROM vault_chunk_vectors v JOIN vault_chunks c ON c.id = v.chunk_id \
         WHERE v.embedding MATCH ?1 AND v.k = ?2 AND v.vault_id IN ({ids}) \
         ORDER BY v.distance"
    )
}

/// The default-surface semantic KNN query (against `chunk_vectors`), used by
/// the evaluation binaries' `semantic_search` entry point.
const DEFAULT_SEMANTIC_KNN_SQL: &str = r#"
    SELECT v.chunk_id, c.note_slug, c.heading_path, c.content, v.distance
    FROM chunk_vectors v
    JOIN chunks c ON c.id = v.chunk_id
    WHERE v.embedding MATCH ?1
      AND v.k = ?2
    ORDER BY v.distance
    "#;

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
    /// Execute the Vault-qualified layered KNN path with an already embedded
    /// query. Progressive callers use this seam to reuse one query vector
    /// across successively larger candidate windows.
    pub(crate) fn vault_semantic_search_layered_with_vector(
        &self,
        conn: &Connection,
        vault_ids: &[VaultId],
        query_vec: &[f32],
        k: usize,
        selection: &LayerSelection,
    ) -> Result<Vec<VaultSemanticHit>, String> {
        if vault_ids.is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        let query_bytes: &[u8] = bytemuck::cast_slice(query_vec);
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
            collect(default_layer_knn_sql(&ids))?;
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

    /// #182 deleted the plan guard along with the legacy query it covered,
    /// leaving the surviving production KNN path with none: a SQL edit that
    /// silently degraded vec0 KNN to a full scan would pass every other test
    /// while making every semantic search scan the whole vector table.
    #[test]
    fn the_vault_qualified_default_query_uses_the_vec0_knn_plan_not_a_full_scan() {
        let cache = SqliteCache::in_memory(384).expect("open");
        let conn = cache.read().expect("read conn");

        let knn_plan: String = conn
            .query_row(
                &format!(
                    "EXPLAIN QUERY PLAN {}",
                    super::default_layer_knn_sql("'12345678-1234-4567-89ab-1234567890ab'")
                ),
                rusqlite::params![bytemuck::cast_slice(&vec![0.0f32; 384]) as &[u8], 10_i64],
                |row| row.get(3),
            )
            .expect("explain vault-qualified knn");
        assert!(
            knn_plan.contains("VIRTUAL TABLE INDEX 0:3"),
            "the default semantic surface must use the vec0 KNN plan (0:3), got: {knn_plan}"
        );

        let scan_plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT v.chunk_id FROM vault_chunk_vectors v \
                 JOIN vault_chunks c ON c.id = v.chunk_id",
                [],
                |row| row.get(3),
            )
            .expect("explain full scan");
        assert!(
            scan_plan.contains("VIRTUAL TABLE INDEX 0:1"),
            "the full-scan shape must be a vec0 fullscan (0:1), so the assertion above \
             distinguishes the two plans, got: {scan_plan}"
        );
    }

    /// #182 deleted the cache-level layer-visibility tests along with the
    /// legacy queries they covered. The vault-qualified path kept demo-mode and
    /// layer-selection coverage higher up, but nothing asserted the rule
    /// directly against the KNN query itself: that the default surface hides a
    /// demoted chunk and `layers=all` returns it.
    #[test]
    fn the_default_surface_hides_a_demoted_chunk_and_layers_all_returns_it() {
        use crate::search::LayerSelection;
        use crate::vault_registry::VaultId;

        let dir = TempDir::new().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("sources")).expect("sources dir");
        std::fs::write(
            dir.path().join("Home.md"),
            "# Home\n\nmelatonin regulates the circadian rhythm",
        )
        .expect("write default note");
        std::fs::write(
            dir.path().join("sources/Clip.md"),
            "# Clip\n\nmelatonin regulates the circadian rhythm",
        )
        .expect("write demoted note");
        std::fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");

        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let vault_id: VaultId = "12345678-1234-4567-89ab-1234567890ab"
            .parse()
            .expect("vault id");
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_vault_snapshot(vault_id, &index, embedder.as_ref())
            .expect("publish snapshot");

        let conn = cache.read().expect("read conn");
        let query_vec = embedder
            .embed(&["melatonin".to_string()])
            .expect("embed")
            .remove(0);

        let slugs = |selection: LayerSelection| -> Vec<String> {
            let mut found = cache
                .vault_semantic_search_layered_with_vector(
                    &conn,
                    &[vault_id],
                    &query_vec,
                    10,
                    &selection,
                )
                .expect("vault semantic search")
                .into_iter()
                .map(|hit| hit.note_slug)
                .collect::<Vec<_>>();
            found.sort();
            found.dedup();
            found
        };

        assert_eq!(
            slugs(LayerSelection::default()),
            vec!["home".to_string()],
            "the default surface must return the undemoted note and hide the demoted one"
        );
        assert_eq!(
            slugs(LayerSelection::All),
            vec!["clip".to_string(), "home".to_string()],
            "layers=all must return the demoted chunk alongside the default surface"
        );
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
