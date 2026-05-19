use rusqlite::OptionalExtension;
use tracing::warn;

use super::SqliteCache;

// Bump this when the schema structure or data-population logic changes to force
// a full cache rebuild on next startup.
const SCHEMA_VERSION: &str = "4";

impl SqliteCache {
    pub fn ensure_schema(&self, embedding_dim: usize) -> Result<(), String> {
        let conn = self.connection()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|error| format!("failed to enable SQLite foreign keys: {error}"))?;

        match existing_schema_version(&conn)? {
            Some(version) if version == SCHEMA_VERSION => {
                create_schema(&conn, embedding_dim)?;
                Ok(())
            }
            Some(version) => {
                warn!(
                    old = %version,
                    new = SCHEMA_VERSION,
                    "SQLite cache schema version mismatch; wiping cache for full rebuild"
                );
                wipe_schema(&conn)?;
                create_schema(&conn, embedding_dim)?;
                Ok(())
            }
            None => {
                create_schema(&conn, embedding_dim)?;
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

fn wipe_schema(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        DROP TABLE IF EXISTS chunk_vectors;
        DROP TABLE IF EXISTS chunks;
        DROP TABLE IF EXISTS headings;
        DROP TABLE IF EXISTS tags;
        DROP TABLE IF EXISTS note_links;
        DROP TABLE IF EXISTS note_fts;
        DROP TABLE IF EXISTS notes;
        DROP TABLE IF EXISTS metadata;
        "#,
    )
    .map_err(|e| format!("failed to wipe stale SQLite cache: {e}"))
}

fn create_schema(conn: &rusqlite::Connection, embedding_dim: usize) -> Result<(), String> {
    let sql = format!(
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

        CREATE VIRTUAL TABLE IF NOT EXISTS chunk_fts USING fts5(
            content,
            content='chunks',
            content_rowid='id',
            tokenize='unicode61 remove_diacritics 2'
        );

        CREATE TRIGGER IF NOT EXISTS chunk_fts_ai AFTER INSERT ON chunks BEGIN
            INSERT INTO chunk_fts(rowid, content) VALUES (new.id, new.content);
        END;

        CREATE TRIGGER IF NOT EXISTS chunk_fts_ad AFTER DELETE ON chunks BEGIN
            INSERT INTO chunk_fts(chunk_fts, rowid, content) VALUES ('delete', old.id, old.content);
        END;

        CREATE TRIGGER IF NOT EXISTS chunk_fts_au AFTER UPDATE ON chunks BEGIN
            INSERT INTO chunk_fts(chunk_fts, rowid, content) VALUES ('delete', old.id, old.content);
            INSERT INTO chunk_fts(rowid, content) VALUES (new.id, new.content);
        END;

        CREATE VIRTUAL TABLE IF NOT EXISTS chunk_vectors USING vec0(
            chunk_id  INTEGER PRIMARY KEY,
            embedding FLOAT[{dim}]
        );

        INSERT INTO metadata(key, value)
        VALUES ('schema_version', '{version}')
        ON CONFLICT(key) DO NOTHING;
        "#,
        dim = embedding_dim,
        version = SCHEMA_VERSION
    );
    conn.execute_batch(&sql)
        .map_err(|error| format!("failed to initialise SQLite cache schema: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::cache::SqliteCache;

    #[test]
    fn fresh_cache_creates_chunks_and_chunk_vectors_tables() {
        let cache = SqliteCache::in_memory(384).expect("open");
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
    fn fresh_cache_records_current_schema_version() {
        let cache = SqliteCache::in_memory(384).expect("open");
        let conn = cache.connection().expect("conn");
        let version: String = conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(version, "4");
    }

    #[test]
    fn fresh_cache_creates_chunk_fts_virtual_table() {
        let cache = SqliteCache::in_memory(384).expect("open");
        let conn = cache.connection().expect("conn");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'chunk_fts'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(count, 1, "chunk_fts virtual table must exist");
    }

    #[test]
    fn chunk_fts_insert_trigger_syncs_new_chunk_rows() {
        let cache = SqliteCache::in_memory(384).expect("open");
        let conn = cache.connection().expect("conn");
        conn.execute(
            "INSERT INTO notes(slug, title, normalized_title, relative_path, normalized_relative_path, absolute_path, content, content_hash, mtime_ns, size_bytes, indexed_at) \
             VALUES ('n1','N1','n1','n1.md','n1.md','/tmp/n1.md','','h',0,0,0)",
            [],
        ).expect("insert note");
        conn.execute(
            "INSERT INTO chunks(note_slug, ordinal, heading_path, content, byte_start, byte_end, content_hash) \
             VALUES ('n1', 0, NULL, 'hello world', 0, 11, 'h0')",
            [],
        ).expect("insert chunk");
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunk_fts WHERE chunk_fts MATCH 'hello'",
                [],
                |row| row.get(0),
            )
            .expect("fts query");
        assert_eq!(hits, 1);
    }

    #[test]
    fn schema_creates_chunk_vectors_with_requested_dim() {
        let cache = SqliteCache::in_memory_with_dim(768).expect("open cache");
        let conn = cache.connection().expect("connection");
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'chunk_vectors'",
                [],
                |row| row.get(0),
            )
            .expect("query sql");
        assert!(sql.contains("FLOAT[768]"), "expected FLOAT[768] in schema, got: {sql}");
    }
}
