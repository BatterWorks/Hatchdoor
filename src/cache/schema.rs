use super::SqliteCache;

impl SqliteCache {
    pub(crate) fn ensure_schema(&self) -> Result<(), String> {
        let conn = self.connection()?;
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
                relative_path TEXT NOT NULL UNIQUE,
                absolute_path TEXT NOT NULL,
                content TEXT NOT NULL,
                content_hash TEXT NOT NULL,
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
                source_slug TEXT NOT NULL,
                target_slug TEXT NOT NULL,
                PRIMARY KEY (source_slug, target_slug)
            );

            CREATE TABLE IF NOT EXISTS headings (
                note_slug TEXT NOT NULL,
                level INTEGER NOT NULL,
                text TEXT NOT NULL,
                anchor TEXT NOT NULL,
                position INTEGER NOT NULL,
                PRIMARY KEY (note_slug, anchor, position)
            );

            CREATE TABLE IF NOT EXISTS tags (
                note_slug TEXT NOT NULL,
                tag TEXT NOT NULL,
                PRIMARY KEY (note_slug, tag)
            );

            INSERT INTO metadata(key, value)
            VALUES ('schema_version', '1')
            ON CONFLICT(key) DO UPDATE SET value = excluded.value;
            "#,
        )
        .map_err(|error| format!("failed to initialise SQLite cache schema: {error}"))?;
        Ok(())
    }
}
