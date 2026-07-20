//! Link, backlink, wikilink-resolution, and graph queries over the cache.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};

use crate::cache::SqliteCache;
use crate::vault::{
    NoteLink, NoteLinks, NoteMetadata, normalize_link_target, normalize_title, slugify,
};

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
    pub metadata: NoteMetadata,
    pub outbound_links: Vec<OutboundLinkRow>,
}

impl SqliteCache {
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
        let conn = self.read()?;

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

    fn note_exists(&self, slug: &str) -> Result<bool, String> {
        let conn = self.read()?;
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM notes WHERE slug = ?1)",
            params![slug],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("failed checking note existence for '{slug}': {error}"))
    }

    fn link_rows(&self, sql: &str, slug: &str) -> Result<Vec<NoteLink>, String> {
        let conn = self.read()?;
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

    pub fn notes_with_outbound_links_batch(
        &self,
        slugs: &[String],
    ) -> Result<HashMap<String, NoteWithLinks>, String> {
        if slugs.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.read()?;

        let placeholders = std::iter::repeat_n("?", slugs.len())
            .collect::<Vec<_>>()
            .join(",");

        let mut map: HashMap<String, NoteWithLinks> = HashMap::new();

        // Note metadata
        let sql_a = format!(
            "SELECT slug, title, relative_path, aliases_json, frontmatter_json \
             FROM notes WHERE slug IN ({placeholders})"
        );
        let mut stmt_a = conn
            .prepare(&sql_a)
            .map_err(|e| format!("prepare notes batch: {e}"))?;
        let rows_a = stmt_a
            .query_map(rusqlite::params_from_iter(slugs.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| format!("query notes batch: {e}"))?;
        for row in rows_a {
            let (slug, title, relative_path, aliases_json, properties_json) =
                row.map_err(|e| format!("read notes batch row: {e}"))?;
            map.insert(
                slug.clone(),
                NoteWithLinks {
                    slug: slug.clone(),
                    title,
                    relative_path,
                    metadata: NoteMetadata {
                        tags: Vec::new(),
                        aliases: serde_json::from_str(&aliases_json)
                            .map_err(|e| format!("invalid cached aliases for '{slug}': {e}"))?,
                        properties: serde_json::from_str(&properties_json)
                            .map_err(|e| format!("invalid cached frontmatter for '{slug}': {e}"))?,
                    },
                    outbound_links: Vec::new(),
                },
            );
        }

        let sql_tags = format!(
            "SELECT note_slug, tag FROM tags WHERE note_slug IN ({placeholders}) \
             ORDER BY note_slug, tag"
        );
        let mut tags_stmt = conn
            .prepare(&sql_tags)
            .map_err(|e| format!("prepare tags batch: {e}"))?;
        let tag_rows = tags_stmt
            .query_map(rusqlite::params_from_iter(slugs.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("query tags batch: {e}"))?;
        for row in tag_rows {
            let (slug, tag) = row.map_err(|e| format!("read tags batch row: {e}"))?;
            if let Some(entry) = map.get_mut(&slug) {
                entry.metadata.tags.push(tag);
            }
        }
        drop(tags_stmt);

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

    pub fn graph_data(&self) -> Result<crate::api_types::GraphResponse, String> {
        use crate::api_types::{GraphEdge, GraphNode, GraphResponse};

        let conn = self.read()?;

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
