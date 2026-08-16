use std::path::Path;

use rusqlite::{OpenFlags, OptionalExtension};
use tracing::warn;

use super::SqliteCache;
use crate::embed::{Embedder, PENDING_IDENTITY};

// Bump this when the schema structure or data-population logic changes to force
// a full cache rebuild on next startup.
const SCHEMA_VERSION: &str = "11";

/// Identify a cache written by a supported single-Vault Hatchdoor release
/// without creating, migrating, or otherwise mutating the database.
pub(crate) fn is_recognized_legacy_cache(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let Ok(connection) = rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return false;
    };
    let version = connection.query_row(
        "SELECT value FROM metadata WHERE key = 'schema_version'",
        [],
        |row| row.get::<_, String>(0),
    );
    let current_version = SCHEMA_VERSION.parse::<u32>().expect("numeric cache schema");
    if !version
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .is_some_and(|version| (1..=current_version).contains(&version))
    {
        return false;
    }

    let hatchdoor_tables = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table' AND name IN ('notes', 'note_links', 'headings', 'tags')",
        [],
        |row| row.get::<_, u32>(0),
    );
    let note_columns = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('notes')
         WHERE name IN (
            'slug', 'title', 'relative_path', 'absolute_path',
            'content', 'content_hash', 'indexed_at'
         )",
        [],
        |row| row.get::<_, u32>(0),
    );
    let hatchdoor_fts = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table' AND name = 'note_fts' AND lower(sql) LIKE '%using fts5%'",
        [],
        |row| row.get::<_, u32>(0),
    );
    matches!(
        (hatchdoor_tables, note_columns, hatchdoor_fts),
        (Ok(4), Ok(7), Ok(1))
    )
}

impl SqliteCache {
    pub fn ensure_schema(&self, embedding_dim: usize) -> Result<(), String> {
        let conn = self.connection()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|error| format!("failed to enable SQLite foreign keys: {error}"))?;

        match existing_schema_version(&conn)? {
            SchemaState::Current => {
                create_schema(&conn, embedding_dim)?;
                Ok(())
            }
            SchemaState::VersionMismatch(version) => {
                warn!(
                    old = %version,
                    new = SCHEMA_VERSION,
                    "SQLite cache schema version mismatch; wiping cache for full rebuild"
                );
                wipe_schema(&conn)?;
                create_schema(&conn, embedding_dim)?;
                Ok(())
            }
            SchemaState::Corrupt(reason) => {
                warn!(
                    reason = %reason,
                    "SQLite cache is half-initialised (likely an interrupted first build); \
                     wiping and rebuilding from Markdown"
                );
                wipe_schema(&conn)?;
                create_schema(&conn, embedding_dim)?;
                Ok(())
            }
            SchemaState::Fresh => {
                create_schema(&conn, embedding_dim)?;
                Ok(())
            }
        }
    }

    /// If the cache already carries a *different* embedder identity than the one
    /// about to build it, wipe and recreate the schema so no vectors from the
    /// previous model survive into the new one's index (mixing embedding spaces
    /// silently ruins semantic search). A cache with no stored identity yet
    /// (fresh, or built by a version that never stamped it) is left alone — the
    /// build stamps the current identity and there is no old model to conflict.
    ///
    /// An embedder whose model has not loaded yet reports
    /// [`PENDING_IDENTITY`], which names no embedding space at all. Comparing
    /// that placeholder against a stored identity would mismatch every time and
    /// destroy a perfectly valid cache on every startup, so it is refused
    /// outright: there is no meaningful index to build without a model, and the
    /// caller is expected to defer the work until setup completes.
    pub fn reset_if_embedder_changed(&self, embedder: &dyn Embedder) -> Result<(), String> {
        let current = embedder.identity();
        if current == PENDING_IDENTITY {
            return Err(
                "embedding model setup is not complete; refusing to evaluate the cache's \
                 embedder identity against a placeholder"
                    .to_string(),
            );
        }
        let stored = match self.get_metadata("embedder_id")? {
            Some(identity) => Some(identity),
            None => {
                let conn = self.connection()?;
                conn.query_row(
                    "SELECT value FROM vault_snapshot_metadata WHERE key = 'embedder_id' LIMIT 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| format!("read Vault snapshot embedder identity: {error}"))?
            }
        };
        if let Some(stored) = stored
            && stored != current
        {
            warn!(
                old = %stored,
                new = %current,
                "embedder identity changed; rebuilding the SQLite cache from scratch"
            );
            let conn = self.connection()?;
            wipe_schema(&conn)?;
            create_schema(&conn, embedder.embedding_dim())?;
        }
        Ok(())
    }
}

/// The state of an existing cache database, as read at startup.
enum SchemaState {
    /// No cache objects yet — a first-time build.
    Fresh,
    /// Fully initialised and on the current schema version.
    Current,
    /// Fully initialised but on an older/newer schema version → rebuild.
    VersionMismatch(String),
    /// Half-initialised (objects but no metadata, or metadata but no
    /// schema_version) — typically an interrupted first build → rebuild.
    Corrupt(String),
}

fn existing_schema_version(conn: &rusqlite::Connection) -> Result<SchemaState, String> {
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
            return Ok(SchemaState::Fresh);
        }
        // Objects but no metadata table: the cache was rebuildable from Markdown
        // anyway, so recover by wiping rather than bricking startup.
        return Ok(SchemaState::Corrupt(
            "cache objects exist but the metadata table is missing".to_string(),
        ));
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
        Some(version) if version == SCHEMA_VERSION => Ok(SchemaState::Current),
        Some(version) => Ok(SchemaState::VersionMismatch(version)),
        None => Ok(SchemaState::Corrupt(
            "metadata table exists but schema_version row is missing".to_string(),
        )),
    }
}

fn wipe_schema(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        DROP TABLE IF EXISTS chunk_vectors_demoted;
        DROP TABLE IF EXISTS chunk_vectors;
        DROP TABLE IF EXISTS vault_chunk_vectors_demoted;
        DROP TABLE IF EXISTS vault_chunk_vectors;
        DROP TRIGGER IF EXISTS vault_chunk_fts_au;
        DROP TRIGGER IF EXISTS vault_chunk_fts_ad;
        DROP TRIGGER IF EXISTS vault_chunk_fts_ai;
        DROP TABLE IF EXISTS vault_chunk_fts;
        DROP TABLE IF EXISTS vault_chunks;
        DROP TABLE IF EXISTS vault_headings;
        DROP TABLE IF EXISTS vault_tags;
        DROP TABLE IF EXISTS vault_note_links;
        DROP TABLE IF EXISTS vault_note_fts;
        DROP TABLE IF EXISTS vault_notes;
        DROP TABLE IF EXISTS vault_snapshot_metadata;
        DROP TABLE IF EXISTS vault_snapshots;
        DROP TRIGGER IF EXISTS chunk_fts_au;
        DROP TRIGGER IF EXISTS chunk_fts_ad;
        DROP TRIGGER IF EXISTS chunk_fts_ai;
        DROP TABLE IF EXISTS chunk_fts;
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

        -- Wrap schema creation in a transaction so it is all-or-nothing: an
        -- interrupted build rolls back to an empty database (a clean "fresh"
        -- state next startup) instead of a half-created one.
        BEGIN;

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
            layer TEXT,
            aliases_json TEXT NOT NULL DEFAULT '[]',
            frontmatter_json TEXT NOT NULL DEFAULT '{{}}',
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

        -- Demoted-layer chunk vectors live in a SEPARATE vec0 table from the
        -- default surface so that default search stays an unfiltered KNN against
        -- `chunk_vectors` (the proven fast path) and never scans a demoted vector.
        -- `layer` is a vec0 PARTITION KEY: a per-layer KNN
        -- (`... WHERE embedding MATCH ? AND k = ? AND layer = ?`) is pushed down to
        -- the matching partition and stays on the KNN plan, so layer separation
        -- never falls back to the Rust full-scan path. One table (not one per
        -- layer) keeps the DDL fixed regardless of the vault's layer names.
        CREATE VIRTUAL TABLE IF NOT EXISTS chunk_vectors_demoted USING vec0(
            chunk_id  INTEGER PRIMARY KEY,
            embedding FLOAT[{dim}],
            layer     TEXT PARTITION KEY
        );

        -- A Vault snapshot is published atomically. These tables deliberately
        -- sit beside the legacy single-Vault cache while the draft backend
        -- packets replace its callers; no implicit or default Vault ID exists.
        CREATE TABLE IF NOT EXISTS vault_snapshots (
            vault_id TEXT PRIMARY KEY,
            participating INTEGER NOT NULL CHECK (participating IN (0, 1)),
            freshness TEXT NOT NULL CHECK (freshness IN ('fresh', 'stale')),
            searchable INTEGER NOT NULL CHECK (searchable IN (0, 1))
        );

        CREATE TABLE IF NOT EXISTS vault_snapshot_metadata (
            vault_id TEXT NOT NULL REFERENCES vault_snapshots(vault_id) ON DELETE CASCADE,
            key TEXT NOT NULL,
            value TEXT NOT NULL,
            PRIMARY KEY (vault_id, key)
        );

        CREATE TABLE IF NOT EXISTS vault_notes (
            vault_id TEXT NOT NULL REFERENCES vault_snapshots(vault_id) ON DELETE CASCADE,
            slug TEXT NOT NULL,
            title TEXT NOT NULL,
            normalized_title TEXT NOT NULL,
            relative_path TEXT NOT NULL,
            normalized_relative_path TEXT NOT NULL,
            absolute_path TEXT NOT NULL,
            content TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            layer TEXT,
            aliases_json TEXT NOT NULL,
            frontmatter_json TEXT NOT NULL,
            mtime_ns INTEGER NOT NULL,
            size_bytes INTEGER NOT NULL,
            indexed_at INTEGER NOT NULL,
            PRIMARY KEY (vault_id, slug),
            UNIQUE (vault_id, relative_path)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS vault_note_fts USING fts5(
            vault_id UNINDEXED,
            title,
            relative_path,
            content,
            slug UNINDEXED,
            tokenize = 'unicode61 remove_diacritics 2'
        );

        CREATE TABLE IF NOT EXISTS vault_note_links (
            vault_id TEXT NOT NULL,
            source_slug TEXT NOT NULL,
            target_slug TEXT NOT NULL,
            PRIMARY KEY (vault_id, source_slug, target_slug),
            FOREIGN KEY (vault_id, source_slug)
                REFERENCES vault_notes(vault_id, slug) ON DELETE CASCADE ON UPDATE CASCADE,
            FOREIGN KEY (vault_id, target_slug)
                REFERENCES vault_notes(vault_id, slug) ON DELETE CASCADE ON UPDATE CASCADE
        );

        CREATE TABLE IF NOT EXISTS vault_headings (
            vault_id TEXT NOT NULL,
            note_slug TEXT NOT NULL,
            level INTEGER NOT NULL,
            text TEXT NOT NULL,
            anchor TEXT NOT NULL,
            position INTEGER NOT NULL,
            PRIMARY KEY (vault_id, note_slug, anchor, position),
            FOREIGN KEY (vault_id, note_slug)
                REFERENCES vault_notes(vault_id, slug) ON DELETE CASCADE ON UPDATE CASCADE
        );

        CREATE TABLE IF NOT EXISTS vault_tags (
            vault_id TEXT NOT NULL,
            note_slug TEXT NOT NULL,
            tag TEXT NOT NULL,
            PRIMARY KEY (vault_id, note_slug, tag),
            FOREIGN KEY (vault_id, note_slug)
                REFERENCES vault_notes(vault_id, slug) ON DELETE CASCADE ON UPDATE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_vault_notes_normalized_title
            ON vault_notes(vault_id, normalized_title);
        CREATE INDEX IF NOT EXISTS idx_vault_notes_normalized_relative_path
            ON vault_notes(vault_id, normalized_relative_path);
        CREATE INDEX IF NOT EXISTS idx_vault_note_links_target
            ON vault_note_links(vault_id, target_slug);
        CREATE INDEX IF NOT EXISTS idx_vault_headings_note
            ON vault_headings(vault_id, note_slug);
        CREATE INDEX IF NOT EXISTS idx_vault_tags_tag
            ON vault_tags(vault_id, tag);

        CREATE TABLE IF NOT EXISTS vault_chunks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            vault_id TEXT NOT NULL,
            note_slug TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            heading_path TEXT,
            content TEXT NOT NULL,
            byte_start INTEGER NOT NULL,
            byte_end INTEGER NOT NULL,
            content_hash TEXT NOT NULL,
            tags TEXT,
            aliases TEXT,
            FOREIGN KEY (vault_id, note_slug)
                REFERENCES vault_notes(vault_id, slug) ON DELETE CASCADE ON UPDATE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_vault_chunks_note
            ON vault_chunks(vault_id, note_slug);
        CREATE INDEX IF NOT EXISTS idx_vault_chunks_content_hash
            ON vault_chunks(vault_id, content_hash);

        CREATE VIRTUAL TABLE IF NOT EXISTS vault_chunk_fts USING fts5(
            vault_id UNINDEXED,
            content,
            content='vault_chunks',
            content_rowid='id',
            tokenize='unicode61 remove_diacritics 2'
        );

        CREATE TRIGGER IF NOT EXISTS vault_chunk_fts_ai AFTER INSERT ON vault_chunks BEGIN
            INSERT INTO vault_chunk_fts(rowid, vault_id, content)
                VALUES (new.id, new.vault_id, new.content);
        END;

        CREATE TRIGGER IF NOT EXISTS vault_chunk_fts_ad AFTER DELETE ON vault_chunks BEGIN
            INSERT INTO vault_chunk_fts(vault_chunk_fts, rowid, vault_id, content)
                VALUES ('delete', old.id, old.vault_id, old.content);
        END;

        CREATE TRIGGER IF NOT EXISTS vault_chunk_fts_au AFTER UPDATE ON vault_chunks BEGIN
            INSERT INTO vault_chunk_fts(vault_chunk_fts, rowid, vault_id, content)
                VALUES ('delete', old.id, old.vault_id, old.content);
            INSERT INTO vault_chunk_fts(rowid, vault_id, content)
                VALUES (new.id, new.vault_id, new.content);
        END;

        CREATE VIRTUAL TABLE IF NOT EXISTS vault_chunk_vectors USING vec0(
            chunk_id INTEGER PRIMARY KEY,
            embedding FLOAT[{dim}],
            vault_id TEXT AUXILIARY
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS vault_chunk_vectors_demoted USING vec0(
            chunk_id INTEGER PRIMARY KEY,
            embedding FLOAT[{dim}],
            layer TEXT PARTITION KEY,
            vault_id TEXT AUXILIARY
        );

        INSERT INTO metadata(key, value)
        VALUES ('schema_version', '{version}')
        ON CONFLICT(key) DO NOTHING;

        COMMIT;
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
    use crate::embed::{RuntimeEmbedder, StubEmbedder};

    /// Regression: startup queued index work before first-run model setup had
    /// installed the embedder, so the identity check compared a real stored
    /// identity against the pending placeholder, mismatched, and wiped the
    /// cache. Every restart paid a full reindex.
    #[test]
    fn pending_embedder_identity_never_wipes_a_valid_cache() {
        let cache = SqliteCache::in_memory(384).expect("open");
        cache
            .set_metadata("embedder_id", "stub-384")
            .expect("stamp a real identity from a previous build");

        let error = cache
            .reset_if_embedder_changed(&RuntimeEmbedder::new())
            .expect_err("an unloaded model must be refused, not treated as a model swap");
        assert!(
            error.contains("setup is not complete"),
            "unexpected error: {error}"
        );

        assert_eq!(
            cache.get_metadata("embedder_id").expect("get").as_deref(),
            Some("stub-384"),
            "the cache must survive the refusal intact"
        );
        let chunks: i64 = cache
            .connection()
            .expect("conn")
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'chunks'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(chunks, 1, "the schema must not have been wiped");
    }

    #[test]
    fn a_genuine_embedder_swap_still_wipes_the_cache() {
        let cache = SqliteCache::in_memory(384).expect("open");
        cache
            .set_metadata("embedder_id", "some-other-model-384")
            .expect("stamp");

        cache
            .reset_if_embedder_changed(&StubEmbedder::new(384))
            .expect("a loaded model with a different identity is a real swap");

        assert_eq!(
            cache.get_metadata("embedder_id").expect("get"),
            None,
            "wiping must clear the previous model's identity along with its vectors"
        );
    }

    #[test]
    fn interrupted_schema_init_rebuilds_instead_of_bricking_startup() {
        // Simulate a crash during first-time init: the metadata table exists but
        // the final schema_version INSERT never committed. On restart this must
        // wipe-and-rebuild, not fail startup forever (which needed a human to
        // delete the cache DB).
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("cache.sqlite3");
        {
            let cache = SqliteCache::open(&path, 384).expect("initial open");
            let conn = cache.connection().expect("conn");
            conn.execute("DELETE FROM metadata WHERE key = 'schema_version'", [])
                .expect("drop schema_version to mimic interrupted init");
        }

        let cache = SqliteCache::open(&path, 384)
            .expect("reopen must rebuild the half-initialised cache, not brick startup");
        let version: Option<String> = cache.get_metadata("schema_version").expect("get");
        assert_eq!(version.as_deref(), Some(super::SCHEMA_VERSION));
    }

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
        assert_eq!(version, super::SCHEMA_VERSION);
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
        assert!(
            sql.contains("FLOAT[768]"),
            "expected FLOAT[768] in schema, got: {sql}"
        );
    }

    #[test]
    fn vault_derived_search_rows_carry_their_vault_identity() {
        let cache = SqliteCache::in_memory(384).expect("open");
        let conn = cache.connection().expect("conn");
        let mut statement = conn
            .prepare("SELECT name FROM pragma_table_info('vault_chunk_fts')")
            .expect("prepare FTS columns");
        let columns = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query FTS columns")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("read FTS columns");
        assert!(
            columns.iter().any(|column| column == "vault_id"),
            "derived FTS rows must remain Vault-qualified"
        );

        for table in ["vault_chunk_vectors", "vault_chunk_vectors_demoted"] {
            let sql: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .expect("vector table DDL");
            assert!(
                sql.contains("vault_id TEXT AUXILIARY"),
                "{table} must retain Vault identity alongside each vector"
            );
        }
    }
}
