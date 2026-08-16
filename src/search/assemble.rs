//! Phase 2 context assembly stage.

use crate::cache::SqliteCache;
use rusqlite::Connection;

use super::{ChunkHit, OutboundLink, SearchResult, project_metadata};

pub fn assemble(
    cache: &SqliteCache,
    hits: Vec<ChunkHit>,
    include_properties: &[String],
) -> Result<Vec<SearchResult>, String> {
    let conn = cache.read()?;
    assemble_on(cache, &conn, hits, include_properties)
}

pub(crate) fn assemble_on(
    cache: &SqliteCache,
    conn: &Connection,
    hits: Vec<ChunkHit>,
    include_properties: &[String],
) -> Result<Vec<SearchResult>, String> {
    if hits.is_empty() {
        return Ok(Vec::new());
    }

    // Preserve first-seen order so we re-attach in stable order later.
    let mut distinct_slugs: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for h in &hits {
        if seen.insert(h.note_slug.clone()) {
            distinct_slugs.push(h.note_slug.clone());
        }
    }

    let metadata = cache.notes_with_outbound_links_batch_on(conn, &distinct_slugs)?;

    let mut out = Vec::with_capacity(hits.len());
    for h in hits {
        let Some(note) = metadata.get(&h.note_slug) else {
            tracing::warn!(
                slug = %h.note_slug,
                "search.assemble: dropping hit whose note vanished between retrieve and assemble"
            );
            continue;
        };
        out.push(SearchResult {
            chunk_id: h.chunk_id,
            note_slug: h.note_slug,
            note_title: note.title.clone(),
            note_path: note.relative_path.clone(),
            heading_path: h.heading_path,
            content: h.content,
            score: h.score,
            layer: note.layer.clone(),
            outbound_links: note
                .outbound_links
                .iter()
                .map(|l| OutboundLink {
                    slug: l.slug.clone(),
                    title: l.title.clone(),
                })
                .collect(),
            metadata: project_metadata(&note.metadata, include_properties),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::search::ChunkHit;
    use crate::vault::VaultIndex;

    use super::assemble;

    fn build_cache(files: &[(&str, &str)]) -> SqliteCache {
        let dir = TempDir::new().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
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
    fn preserves_hit_order() {
        let cache = build_cache(&[
            ("Alpha.md", "# Alpha\n\nbody"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let hits = vec![
            ChunkHit {
                chunk_id: 1,
                note_slug: "bravo".to_string(),
                heading_path: None,
                content: "b body".to_string(),
                score: 0.9,
            },
            ChunkHit {
                chunk_id: 2,
                note_slug: "alpha".to_string(),
                heading_path: None,
                content: "a body".to_string(),
                score: 0.8,
            },
        ];
        let out = assemble(&cache, hits, &[]).expect("assemble");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].note_slug, "bravo");
        assert_eq!(out[1].note_slug, "alpha");
    }

    #[test]
    fn drops_hits_whose_note_vanished() {
        let cache = build_cache(&[("Alpha.md", "# Alpha\n\nbody")]);
        let hits = vec![
            ChunkHit {
                chunk_id: 1,
                note_slug: "alpha".to_string(),
                heading_path: None,
                content: "a".to_string(),
                score: 0.9,
            },
            ChunkHit {
                chunk_id: 2,
                note_slug: "ghost".to_string(),
                heading_path: None,
                content: "g".to_string(),
                score: 0.8,
            },
        ];
        let out = assemble(&cache, hits, &[]).expect("assemble");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].note_slug, "alpha");
    }

    #[test]
    fn attaches_resolved_outbound_links() {
        let cache = build_cache(&[
            ("Alpha.md", "# Alpha\n\nlinks to [[Bravo]] and [[Ghost]]"),
            ("Bravo.md", "# Bravo\n\nbody"),
        ]);
        let hits = vec![ChunkHit {
            chunk_id: 1,
            note_slug: "alpha".to_string(),
            heading_path: None,
            content: "a body".to_string(),
            score: 0.9,
        }];
        let out = assemble(&cache, hits, &[]).expect("assemble");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].outbound_links.len(), 1);
        assert_eq!(out[0].outbound_links[0].slug, "bravo");
    }
}
