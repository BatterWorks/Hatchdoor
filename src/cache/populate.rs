use std::fs;

use rusqlite::params;

use crate::vault::VaultIndex;

use super::parse::{content_hash, current_unix_timestamp, extract_headings, extract_tags};
use super::SqliteCache;

impl SqliteCache {
    pub(crate) fn replace_from_index(&self, index: &VaultIndex) -> Result<(), String> {
        let entries = index.ordered_entries();
        let now = current_unix_timestamp();
        let mut conn = self.connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("failed to start SQLite cache refresh: {error}"))?;

        tx.execute_batch(
            r#"
            DELETE FROM note_fts;
            DELETE FROM note_links;
            DELETE FROM headings;
            DELETE FROM tags;
            DELETE FROM notes;
            "#,
        )
        .map_err(|error| format!("failed to clear SQLite cache tables: {error}"))?;

        for entry in &entries {
            let content = fs::read_to_string(&entry.path).map_err(|error| {
                format!("failed reading note '{}': {error}", entry.path.display())
            })?;
            let hash = content_hash(&content);
            let absolute_path = entry.path.to_string_lossy().to_string();
            tx.execute(
                r#"
                INSERT INTO notes(slug, title, relative_path, absolute_path, content, content_hash, indexed_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    &entry.slug,
                    &entry.title,
                    &entry.relative_path,
                    &absolute_path,
                    &content,
                    &hash,
                    now,
                ],
            )
            .map_err(|error| format!("failed to insert note '{}': {error}", entry.slug))?;
            let note_id = tx.last_insert_rowid();
            tx.execute(
                r#"
                INSERT INTO note_fts(rowid, title, relative_path, content, slug)
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![note_id, &entry.title, &entry.relative_path, &content, &entry.slug],
            )
            .map_err(|error| format!("failed to index note '{}' for search: {error}", entry.slug))?;

            for heading in extract_headings(&content) {
                tx.execute(
                    r#"
                    INSERT OR IGNORE INTO headings(note_slug, level, text, anchor, position)
                    VALUES (?1, ?2, ?3, ?4, ?5)
                    "#,
                    params![&entry.slug, heading.level, &heading.text, &heading.anchor, heading.position],
                )
                .map_err(|error| format!("failed to cache heading for '{}': {error}", entry.slug))?;
            }

            for tag in extract_tags(&content) {
                tx.execute(
                    r#"
                    INSERT OR IGNORE INTO tags(note_slug, tag)
                    VALUES (?1, ?2)
                    "#,
                    params![&entry.slug, &tag],
                )
                .map_err(|error| format!("failed to cache tag for '{}': {error}", entry.slug))?;
            }
        }

        for entry in &entries {
            if let Some(links) = index.note_links(&entry.slug) {
                for link in links.outgoing {
                    tx.execute(
                        r#"
                        INSERT OR IGNORE INTO note_links(source_slug, target_slug)
                        VALUES (?1, ?2)
                        "#,
                        params![&entry.slug, &link.slug],
                    )
                    .map_err(|error| format!("failed to cache link for '{}': {error}", entry.slug))?;
                }
            }
        }

        tx.commit()
            .map_err(|error| format!("failed to commit SQLite cache refresh: {error}"))?;
        Ok(())
    }
}
