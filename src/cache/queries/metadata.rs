//! Note metadata queries over the SQLite cache.

use rusqlite::{OptionalExtension, params};

use crate::cache::SqliteCache;
use crate::vault::{Note, NoteMetadata};

impl SqliteCache {
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
}
