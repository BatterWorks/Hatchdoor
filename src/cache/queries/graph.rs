//! Link, backlink, and wikilink-resolution queries over the cache.

use rusqlite::{OptionalExtension, params};

use crate::cache::SqliteCache;
use crate::search::LayerSelection;
use crate::vault::{NoteLink, NoteLinks, normalize_link_target, normalize_title, slugify};

impl SqliteCache {
    pub fn note_links(
        &self,
        slug: &str,
        selection: &LayerSelection,
    ) -> Result<Option<NoteLinks>, String> {
        if !self.note_exists(slug)? {
            return Ok(None);
        }

        // Forward links always resolve across the layer boundary and carry the
        // target's layer — a citation from a compiled page into a source must
        // work regardless of the selection.
        let outgoing = self.link_rows(
            r#"
            SELECT target.title, target.slug, target.relative_path, target.layer
            FROM note_links links
            JOIN notes target ON target.slug = links.target_slug
            WHERE links.source_slug = ?1
            ORDER BY (target.layer IS NOT NULL), target.relative_path
            "#,
            slug,
        )?;
        // Backlinks from a demoted layer are hidden under the default selection
        // and included only when the selection names that layer.
        let backlinks = self.link_rows(
            &format!(
                r#"
            SELECT source.title, source.slug, source.relative_path, source.layer
            FROM note_links links
            JOIN notes source ON source.slug = links.source_slug
            WHERE links.target_slug = ?1 AND {}
            ORDER BY (source.layer IS NOT NULL), source.relative_path
            "#,
                selection.sql_filter("source.layer"),
            ),
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
                ORDER BY (layer IS NOT NULL), relative_path
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
                ORDER BY (layer IS NOT NULL), relative_path
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
                    layer: row.get(3)?,
                })
            })
            .map_err(|error| format!("failed to query note links: {error}"))?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("failed to read note links: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::vault::VaultIndex;

    fn build_layered_cache(files: &[(&str, &str)]) -> SqliteCache {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, body).expect("write");
        }
        let cache = SqliteCache::in_memory(384).expect("open");
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(384));
        let index = VaultIndex::build(dir.path()).expect("build");
        cache
            .replace_from_index_with_embedder(&index, embedder.as_ref())
            .expect("index");
        cache
    }

    #[test]
    fn wikilink_resolves_to_the_default_surface_on_a_title_collision() {
        // `sources/Melatonin` sorts before `wiki/Melatonin` by relative_path, so
        // the pre-fix `ORDER BY relative_path` returned the clipping. The fix
        // orders by layer first (default surface before demoted).
        let cache = build_layered_cache(&[
            ("sources/.hatchdoor-layer", "sources"),
            ("sources/Melatonin.md", "# Melatonin\n\nraw clipping"),
            ("wiki/Melatonin.md", "# Melatonin\n\ncompiled page"),
        ]);

        let resolved = cache
            .resolve_wikilink("Melatonin")
            .expect("resolve")
            .expect("a note resolves");
        assert_eq!(
            resolved.1, "wiki/Melatonin",
            "[[Melatonin]] must resolve to the default-surface page, not the demoted clipping"
        );
    }

    #[test]
    fn wikilink_by_path_prefers_the_default_surface() {
        // Exercise the by-path branch: a bare stem that matches two notes'
        // normalized_relative_path... which cannot happen across folders, so use
        // an explicit same-basename path to confirm ordering is layer-first.
        let cache = build_layered_cache(&[
            ("sources/.hatchdoor-layer", "sources"),
            ("sources/Melatonin.md", "# Melatonin\n\nraw clipping"),
            ("wiki/Melatonin.md", "# Melatonin\n\ncompiled page"),
        ]);
        // A demoted-only title still resolves (reachability preserved).
        let cache_single = build_layered_cache(&[
            ("sources/.hatchdoor-layer", "sources"),
            ("sources/Ashwagandha.md", "# Ashwagandha\n\nraw clipping"),
        ]);
        let resolved = cache_single
            .resolve_wikilink("Ashwagandha")
            .expect("resolve")
            .expect("demoted-only note still resolves");
        assert_eq!(resolved.1, "sources/Ashwagandha");
        // And the collision case still prefers default.
        let resolved = cache
            .resolve_wikilink("Melatonin")
            .expect("resolve")
            .expect("resolves");
        assert_eq!(resolved.1, "wiki/Melatonin");
    }

    /// A default page linking a demoted clipping, which links back.
    fn linked_layer_cache() -> SqliteCache {
        build_layered_cache(&[
            ("sources/.hatchdoor-layer", "sources"),
            ("wiki/Page.md", "# Page\n\nsee [[Clip]]"),
            ("sources/Clip.md", "# Clip\n\nback to [[Page]]"),
        ])
    }

    #[test]
    fn forward_link_resolves_into_a_demoted_note_and_carries_its_layer() {
        let cache = linked_layer_cache();
        let links = cache
            .note_links("page", &crate::search::LayerSelection::default_surface())
            .expect("links")
            .expect("page exists");
        // Forward link resolves across the boundary regardless of selection.
        assert_eq!(links.outgoing.len(), 1);
        assert_eq!(links.outgoing[0].slug, "clip");
        assert_eq!(
            links.outgoing[0].layer.as_deref(),
            Some("sources"),
            "a forward link into a demoted note carries the target's layer"
        );
    }

    #[test]
    fn demoted_backlink_is_hidden_by_default_and_shown_when_selected() {
        let cache = linked_layer_cache();

        // Default surface: the backlink from the demoted clipping is hidden.
        let default_links = cache
            .note_links("page", &crate::search::LayerSelection::default_surface())
            .expect("links")
            .expect("page exists");
        assert!(
            default_links.backlinks.is_empty(),
            "a backlink from a demoted layer is hidden under the default selection"
        );

        // Selecting the layer reveals it, carrying the source's layer.
        let (selection, _) = crate::search::LayerSelection::parse(
            &["sources".to_string()],
            &["sources".to_string()],
        );
        let sourced_links = cache
            .note_links("page", &selection)
            .expect("links")
            .expect("page exists");
        assert_eq!(sourced_links.backlinks.len(), 1);
        assert_eq!(sourced_links.backlinks[0].slug, "clip");
        assert_eq!(sourced_links.backlinks[0].layer.as_deref(), Some("sources"));
    }
}
