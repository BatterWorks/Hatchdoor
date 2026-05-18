use rusqlite::OptionalExtension;

use super::SqliteCache;

const SCHEMA_VERSION: &str = "3";

impl SqliteCache {
    pub(crate) fn ensure_schema(&self) -> Result<(), String> {
        let conn = self.connection()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|error| format!("failed to enable SQLite foreign keys: {error}"))?;

        match existing_schema_version(&conn)? {
            Some(version) if version == SCHEMA_VERSION => {
                create_schema(&conn)?;
                Ok(())
            }
            Some(version) => Err(format!(
                "unsupported SQLite cache schema version '{version}' for expected version '{SCHEMA_VERSION}'. Delete the cache DB and restart Hatchdoor to rebuild it from Markdown."
            )),
            None => {
                create_schema(&conn)?;
                Ok(())
            }
        }
    }
}

fn existing_schema_version(conn: &rusqlite::Connection) -> Result<Option<String>, String> {
    let metadata_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'metadata')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("failed checking SQLite metadata table: {error}"))?;

    if !metadata_exists {
        let object_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table', 'index', 'trigger', 'view') AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| format!("failed checking SQLite cache objects: {error}"))?;
        if object_count == 0 {
            return Ok(None);
        }
        return Err(
            "SQLite cache contains objects but no schema metadata. Delete the cache DB and restart Hatchdoor to rebuild it from Markdown."
                .to_string(),
        );
    }

    let version = conn
        .query_row(
            "SELECT value FROM metadata WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("failed reading SQLite cache schema version: {error}"))?;

    match version {
        Some(version) => Ok(Some(version)),
        None => Err(
            "SQLite cache metadata exists but schema_version is missing. Delete the cache DB and restart Hatchdoor to rebuild it from Markdown."
                .to_string(),
        ),
    }
}

fn create_schema(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS notes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            slug TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            normalized_title TEXT NOT NULL,
            relative_path TEXT NOT NULL UNIQUE,
            normalized_relative_path TEXT NOT NULL,
            absolute_path TEXT NOT NULL,
            content TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            mtime_ns INTEGER NOT NULL,
            size_bytes INTEGER NOT NULL,
            indexed_at INTEGER NOT NULL
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS note_fts USING fts5(
            title,
            relative_path,
            content,
            slug UNINDEXED,
            tokenize = 'unicode61 remove_diacritics 2'
        );

        CREATE TABLE IF NOT EXISTS note_links (
            source_slug TEXT NOT NULL REFERENCES notes(slug) ON DELETE CASCADE ON UPDATE CASCADE,
            target_slug TEXT NOT NULL REFERENCES notes(slug) ON DELETE CASCADE ON UPDATE CASCADE,
            PRIMARY KEY (source_slug, target_slug)
        );

        CREATE TABLE IF NOT EXISTS headings (
            note_slug TEXT NOT NULL REFERENCES notes(slug) ON DELETE CASCADE ON UPDATE CASCADE,
            level INTEGER NOT NULL,
            text TEXT NOT NULL,
            anchor TEXT NOT NULL,
            position INTEGER NOT NULL,
            PRIMARY KEY (note_slug, anchor, position)
        );

        CREATE TABLE IF NOT EXISTS tags (
            note_slug TEXT NOT NULL REFERENCES notes(slug) ON DELETE CASCADE ON UPDATE CASCADE,
            tag TEXT NOT NULL,
            PRIMARY KEY (note_slug, tag)
        );

        CREATE INDEX IF NOT EXISTS idx_notes_normalized_title
            ON notes(normalized_title);
        CREATE INDEX IF NOT EXISTS idx_notes_normalized_relative_path
            ON notes(normalized_relative_path);
        CREATE INDEX IF NOT EXISTS idx_note_links_target_slug
            ON note_links(target_slug);
        CREATE INDEX IF NOT EXISTS idx_headings_note_slug
            ON headings(note_slug);
        CREATE INDEX IF NOT EXISTS idx_tags_tag
            ON tags(tag);

        CREATE TABLE IF NOT EXISTS chunks (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            note_slug    TEXT    NOT NULL REFERENCES notes(slug) ON DELETE CASCADE ON UPDATE CASCADE,
            ordinal      INTEGER NOT NULL,
            heading_path TEXT,
            content      TEXT    NOT NULL,
            byte_start   INTEGER NOT NULL,
            byte_end     INTEGER NOT NULL,
            content_hash TEXT    NOT NULL,
            tags         TEXT,
            aliases      TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_chunks_note_slug ON chunks(note_slug);
        CREATE INDEX IF NOT EXISTS idx_chunks_content_hash ON chunks(content_hash);

        CREATE VIRTUAL TABLE IF NOT EXISTS chunk_vectors USING vec0(
            chunk_id  INTEGER PRIMARY KEY,
            embedding FLOAT[384]
        );

        INSERT INTO metadata(key, value)
        VALUES ('schema_version', '3')
        ON CONFLICT(key) DO NOTHING;
        "#,
    )
    .map_err(|error| format!("failed to initialise SQLite cache schema: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::cache::SqliteCache;

    #[test]
    fn fresh_cache_creates_chunks_and_chunk_vectors_tables() {
        let cache = SqliteCache::in_memory().expect("open");
        let conn = cache.connection().expect("conn");

        let chunks: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'chunks'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(chunks, 1, "chunks table must exist");

        let chunk_vectors: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'chunk_vectors'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(chunk_vectors, 1, "chunk_vectors virtual table must exist");
    }

    #[test]
    fn fresh_cache_records_schema_version_3() {
        let cache = SqliteCache::in_memory().expect("open");
        let conn = cache.connection().expect("conn");
        let version: String = conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(version, "3");
    }
}
