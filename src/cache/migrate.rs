//! The one cache migration ADR-50 allows: from the previous schema version to
//! the current one, keeping the vectors instead of re-embedding every chunk.
//! Delete this migration when the next schema change lands; a cache two
//! versions old is wiped and rebuilt, as before.
//!
//! Version 12 to 13 (#634): the keyword index tokenized Chinese, Japanese,
//! Korean and Thai as whole clauses, so a word inside a sentence never
//! matched. Version 13 indexes those scripts as overlapping pairs, read from
//! a new `vault_chunks.keyword_text` column (`keyword::keyword_index_text`).
//! The migration fills that column from the chunk text the cache already
//! holds and rebuilds only the keyword index from it. No vector table is
//! written, so their rows cannot change; the row-count check proves no
//! cascade touched them.

use std::collections::BTreeMap;

use rusqlite::{Connection, Transaction, params};

use super::keyword::keyword_index_text;
use super::schema::{SCHEMA_VERSION, vault_chunk_keyword_index_sql};

pub(super) const PREVIOUS_SCHEMA_VERSION: &str = "12";

const VECTOR_TABLES: [&str; 2] = ["vault_chunk_vectors", "vault_chunk_vectors_demoted"];

/// Run the whole migration in one transaction and commit only after every
/// check has passed. The error names the step or check that failed. On error
/// the caller wipes the cache rather than trusting the rollback (ADR-50).
pub(super) fn migrate_from_previous_schema(
    conn: &mut Connection,
    _embedding_dim: usize,
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|error| format!("start migration: {error}"))?;

    let mut before = Vec::new();
    for table in VECTOR_TABLES {
        before.push(vectors_per_vault(&tx, table)?);
    }

    tx.execute_batch(
        "DROP TRIGGER vault_chunk_fts_ai;
         DROP TRIGGER vault_chunk_fts_ad;
         DROP TRIGGER vault_chunk_fts_au;
         DROP TABLE vault_chunk_fts;
         ALTER TABLE vault_chunks ADD COLUMN keyword_text TEXT;",
    )
    .map_err(|error| format!("drop the old keyword index: {error}"))?;
    fill_keyword_text(&tx)?;
    tx.execute_batch(&vault_chunk_keyword_index_sql())
        .map_err(|error| format!("create the new keyword index: {error}"))?;
    tx.execute(
        "INSERT INTO vault_chunk_fts(vault_chunk_fts) VALUES ('rebuild')",
        [],
    )
    .map_err(|error| format!("rebuild the keyword index: {error}"))?;

    for (table, before) in VECTOR_TABLES.into_iter().zip(&before) {
        let after = vectors_per_vault(&tx, table)?;
        if &after != before {
            return Err(format!(
                "row count check failed on {table}: {before:?} before, {after:?} after"
            ));
        }
    }
    check_keyword_index(&tx)?;

    let integrity: String = tx
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| format!("integrity check failed to run: {error}"))?;
    if integrity != "ok" {
        return Err(format!("integrity check failed: {integrity}"));
    }

    tx.execute(
        "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
        params![SCHEMA_VERSION],
    )
    .map_err(|error| format!("stamp the schema version: {error}"))?;
    tx.commit()
        .map_err(|error| format!("commit migration: {error}"))
}

/// Vector rows per Vault key in `table`.
fn vectors_per_vault(tx: &Transaction<'_>, table: &str) -> Result<BTreeMap<i64, i64>, String> {
    let mut statement = tx
        .prepare(&format!(
            "SELECT vault_key, COUNT(*) FROM {table} GROUP BY vault_key"
        ))
        .map_err(|error| format!("count rows in {table}: {error}"))?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|error| format!("count rows in {table}: {error}"))?;
    rows.collect::<rusqlite::Result<_>>()
        .map_err(|error| format!("count rows in {table}: {error}"))
}

/// Store the keyword text of every chunk whose text the keyword index reads
/// differently. Most chunks have none and keep `NULL`.
fn fill_keyword_text(tx: &Transaction<'_>) -> Result<(), String> {
    let mut read = tx
        .prepare("SELECT id, content FROM vault_chunks")
        .map_err(|error| format!("read chunk text: {error}"))?;
    let mut write = tx
        .prepare("UPDATE vault_chunks SET keyword_text = ?2 WHERE id = ?1")
        .map_err(|error| format!("prepare keyword text: {error}"))?;
    let mut rows = read
        .query([])
        .map_err(|error| format!("read chunk text: {error}"))?;
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("read chunk text: {error}"))?
    {
        let id: i64 = row
            .get(0)
            .map_err(|error| format!("read chunk text: {error}"))?;
        let content: String = row
            .get(1)
            .map_err(|error| format!("read chunk text: {error}"))?;
        if let Some(text) = keyword_index_text(&content) {
            write
                .execute(params![id, text])
                .map_err(|error| format!("store keyword text: {error}"))?;
        }
    }
    Ok(())
}

/// The rebuilt index must hold exactly one entry per chunk and agree with
/// the text it was built from.
fn check_keyword_index(tx: &Transaction<'_>) -> Result<(), String> {
    let (chunks, indexed): (i64, i64) = tx
        .query_row(
            "SELECT (SELECT COUNT(*) FROM vault_chunks),
                    (SELECT COUNT(*) FROM vault_chunk_fts_docsize)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("count keyword index rows: {error}"))?;
    if chunks != indexed {
        return Err(format!(
            "keyword index check failed: {indexed} entries for {chunks} chunks"
        ));
    }
    tx.execute(
        "INSERT INTO vault_chunk_fts(vault_chunk_fts, rank) VALUES ('integrity-check', 1)",
        [],
    )
    .map_err(|error| format!("keyword index check failed: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::{TempDir, tempdir};

    use crate::cache::SqliteCache;
    use crate::embed::{Embedder, StubEmbedder};
    use crate::search::LayerSelection;
    use crate::vault::VaultIndex;
    use crate::vault_registry::VaultId;

    const VAULTS: [&str; 3] = [
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
        "33333333-3333-4333-8333-333333333333",
    ];
    const QUERIES: [&str; 4] = ["melatonin sleep", "garden tomatoes", "rust borrow", "notes"];

    /// The version 12 keyword index, verbatim.
    const PREVIOUS_LAYOUT: &str = "
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
        END;";

    fn vault_ids() -> Vec<VaultId> {
        VAULTS
            .iter()
            .map(|id| id.parse().expect("Vault ID"))
            .collect()
    }

    fn write_vault(root: &Path, topic: &str) {
        std::fs::create_dir_all(root.join("sources")).expect("sources");
        std::fs::write(root.join("sources/.hatchdoor-layer"), "sources").expect("marker");
        for (path, body) in [
            ("Sleep.md", "# Sleep\n\nmelatonin regulates sleep"),
            (
                "Garden.md",
                "# Garden\n\n## Tomatoes\n\ngarden tomatoes need sun",
            ),
            ("Rust.md", "# Rust\n\nthe borrow checker in rust"),
            ("sources/Clip.md", "# Clip\n\nmelatonin and garden notes"),
            (
                "日本語のノート.md",
                "# 日本語のノート\n\nひらがな、カタカナ、漢字が混ざった本文です。",
            ),
        ] {
            std::fs::write(root.join(path), format!("{body}\n\n{topic} notes")).expect("note");
        }
    }

    /// A cache file in the version 12 layout holding three published Vaults,
    /// with demoted vectors. Built by publishing with today's code, then
    /// putting the old keyword index back with the old DDL.
    fn previous_version_cache() -> (TempDir, std::path::PathBuf) {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("cache.sqlite3");
        let embedder = StubEmbedder::new(384);
        {
            let cache = SqliteCache::open(&path, 384).expect("open");
            for (n, vault_id) in vault_ids().into_iter().enumerate() {
                let root = directory.path().join(format!("vault-{n}"));
                write_vault(&root, &format!("topic{n}"));
                let index = VaultIndex::build(&root).expect("index");
                cache
                    .replace_vault_snapshot(vault_id, &index, &embedder)
                    .expect("publish");
            }
            let conn = cache.connection().expect("conn");
            conn.execute_batch(
                "DROP TRIGGER vault_chunk_fts_ai;
                 DROP TRIGGER vault_chunk_fts_ad;
                 DROP TRIGGER vault_chunk_fts_au;
                 DROP TABLE vault_chunk_fts;
                 DROP VIEW vault_chunk_keyword_source;
                 ALTER TABLE vault_chunks DROP COLUMN keyword_text;",
            )
            .expect("remove the new keyword index");
            conn.execute_batch(PREVIOUS_LAYOUT).expect("old layout");
            conn.execute_batch(
                "INSERT INTO vault_chunk_fts(vault_chunk_fts) VALUES ('rebuild');
                 UPDATE metadata SET value = '12' WHERE key = 'schema_version';
                 UPDATE vault_snapshot_metadata SET value = '12'
                     WHERE key = 'schema_version';",
            )
            .expect("fill the old layout");
            let demoted: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM vault_chunk_vectors_demoted",
                    [],
                    |row| row.get(0),
                )
                .expect("count");
            assert!(demoted > 0, "the fixture must exercise the demoted table");
        }
        (directory, path)
    }

    /// The fixture's embedder, counting what it is asked to embed.
    #[derive(Default)]
    struct CountingStub {
        embedded: std::sync::atomic::AtomicUsize,
    }
    impl Embedder for CountingStub {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            self.embedded
                .fetch_add(texts.len(), std::sync::atomic::Ordering::Relaxed);
            StubEmbedder::new(384).embed(texts)
        }
        fn embedding_dim(&self) -> usize {
            384
        }
        fn identity(&self) -> String {
            StubEmbedder::new(384).identity()
        }
        fn token_count(&self, text: &str, add: bool) -> Result<usize, String> {
            StubEmbedder::new(384).token_count(text, add)
        }
    }

    type Hit = (String, i64, String, f32);

    fn semantic_search(
        cache: &SqliteCache,
        vault_ids: &[VaultId],
        query: &str,
        all_layers: bool,
    ) -> Vec<Hit> {
        let vector = StubEmbedder::new(384)
            .embed(&[query.to_string()])
            .expect("embed")
            .remove(0);
        let selection = if all_layers {
            LayerSelection::All
        } else {
            LayerSelection::default()
        };
        let conn = cache.read().expect("read");
        cache
            .vault_semantic_search_layered_with_vector(&conn, vault_ids, &vector, 5, &selection)
            .expect("search")
            .into_iter()
            .map(|hit| {
                (
                    hit.vault_id.to_string(),
                    hit.chunk_id,
                    hit.note_slug,
                    hit.distance,
                )
            })
            .collect()
    }

    fn keyword_search(cache: &SqliteCache, query: &str) -> Vec<(String, i64, String)> {
        let conn = cache.read().expect("read");
        cache
            .vault_fts_search_chunks(&conn, &vault_ids(), query, &LayerSelection::All, 20)
            .expect("keyword search")
            .into_iter()
            .map(|hit| (hit.vault_id.to_string(), hit.chunk_id, hit.note_slug))
            .collect()
    }

    /// Every scope and layer selection the fixed searches run under.
    fn fixed_searches(cache: &SqliteCache) -> Vec<Vec<Hit>> {
        let ids = vault_ids();
        let mut scopes = vec![ids.clone()];
        scopes.extend(ids.iter().map(|id| vec![*id]));
        let mut results = Vec::new();
        for scope in &scopes {
            for query in QUERIES {
                for all_layers in [false, true] {
                    results.push(semantic_search(cache, scope, query, all_layers));
                }
            }
        }
        results
    }

    fn vector_rows(cache: &SqliteCache) -> (i64, i64) {
        cache
            .connection()
            .expect("conn")
            .query_row(
                "SELECT (SELECT COUNT(*) FROM vault_chunk_vectors),
                        (SELECT COUNT(*) FROM vault_chunk_vectors_demoted)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("count vectors")
    }

    fn published_vaults(cache: &SqliteCache) -> i64 {
        cache
            .connection()
            .expect("conn")
            .query_row("SELECT COUNT(*) FROM vault_snapshots", [], |row| row.get(0))
            .expect("count")
    }

    fn schema_version(cache: &SqliteCache) -> Option<String> {
        cache.get_metadata("schema_version").expect("version")
    }

    #[test]
    fn a_migrated_cache_finds_words_inside_cjk_sentences_without_re_embedding() {
        let (directory, path) = previous_version_cache();
        let cache = SqliteCache::open(&path, 384).expect("open the previous version");
        assert_eq!(
            schema_version(&cache).as_deref(),
            Some(super::PREVIOUS_SCHEMA_VERSION),
            "opening must leave the cache alone until the model check has run"
        );
        let semantic_before = fixed_searches(&cache);
        assert!(semantic_before.iter().all(|hits| !hits.is_empty()));
        let latin_before = keyword_search(&cache, "melatonin");
        assert_eq!(latin_before.len(), 6);
        assert!(
            keyword_search(&cache, "本文").is_empty(),
            "the old index holds the whole clause as one token"
        );
        let vectors_before = vector_rows(&cache);

        cache
            .prepare_for_embedder(&StubEmbedder::new(384))
            .expect("loading the model migrates the cache");
        cache
            .reset_if_embedder_changed(&StubEmbedder::new(384))
            .expect("the next Index turn's model check finds nothing to migrate");

        assert_eq!(
            schema_version(&cache).as_deref(),
            Some(super::SCHEMA_VERSION)
        );
        assert_eq!(published_vaults(&cache), 3, "migration must not wipe");
        assert_eq!(vector_rows(&cache), vectors_before);
        assert_eq!(fixed_searches(&cache), semantic_before);
        assert_eq!(keyword_search(&cache, "melatonin"), latin_before);
        for word in ["本文", "漢字", "日本語", "本"] {
            let hits = keyword_search(&cache, word);
            assert_eq!(hits.len(), 3, "{word}: {hits:?}");
            assert!(hits.iter().all(|hit| hit.2 == "日本語のノート"), "{word}");
        }

        let embedder = CountingStub::default();
        let index = VaultIndex::build(directory.path().join("vault-0")).expect("index");
        cache
            .replace_vault_snapshot(vault_ids()[0], &index, &embedder)
            .expect("republish after migrating");
        assert_eq!(
            embedder.embedded.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "an unchanged Vault must reuse every migrated vector"
        );
        assert_eq!(keyword_search(&cache, "本文").len(), 3);
    }

    #[test]
    fn a_failed_step_names_itself_and_commits_nothing() {
        let (_directory, path) = previous_version_cache();
        let cache = SqliteCache::open(&path, 384).expect("open");
        let mut conn = cache.connection().expect("conn");
        conn.execute_batch("DROP TRIGGER vault_chunk_fts_au;")
            .expect("a half-built keyword index");

        let failure = super::migrate_from_previous_schema(&mut conn, 384)
            .expect_err("a missing trigger must fail the migration");
        assert!(failure.contains("drop the old keyword index"), "{failure}");
        let (version, column, index): (String, bool, bool) = conn
            .query_row(
                "SELECT (SELECT value FROM metadata WHERE key = 'schema_version'),
                        EXISTS(SELECT 1 FROM pragma_table_info('vault_chunks')
                               WHERE name = 'keyword_text'),
                        EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'vault_chunk_fts')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read the cache after the failure");
        assert_eq!(
            version,
            super::PREVIOUS_SCHEMA_VERSION,
            "nothing may commit"
        );
        assert!(!column, "the new column must be rolled back");
        assert!(
            index,
            "the old keyword index must be back after the rollback"
        );
    }

    #[test]
    fn a_failed_migration_wipes_and_rebuilds_a_searchable_cache() {
        let (directory, path) = previous_version_cache();
        let cache = SqliteCache::open(&path, 384).expect("open");
        cache
            .connection()
            .expect("conn")
            .execute_batch("DROP TRIGGER vault_chunk_fts_au;")
            .expect("a half-built keyword index");

        let embedder = StubEmbedder::new(384);
        cache
            .reset_if_embedder_changed(&embedder)
            .expect("a failed migration recovers by wiping");
        assert_eq!(
            published_vaults(&cache),
            0,
            "the cache must have been wiped"
        );
        assert_eq!(
            schema_version(&cache).as_deref(),
            Some(super::SCHEMA_VERSION)
        );

        let vault_id = vault_ids()[0];
        let index = VaultIndex::build(directory.path().join("vault-0")).expect("index");
        cache
            .replace_vault_snapshot(vault_id, &index, &embedder)
            .expect("rebuild");
        assert!(!semantic_search(&cache, &[vault_id], "melatonin sleep", true).is_empty());
        assert_eq!(keyword_search(&cache, "本文").len(), 1);
    }

    #[test]
    fn a_cache_two_versions_old_is_wiped_not_migrated() {
        let (_directory, path) = previous_version_cache();
        {
            let cache = SqliteCache::open(&path, 384).expect("open");
            cache
                .connection()
                .expect("conn")
                .execute(
                    "UPDATE metadata SET value = '11' WHERE key = 'schema_version'",
                    [],
                )
                .expect("age the cache");
        }
        let cache = SqliteCache::open(&path, 384).expect("reopen");
        assert_eq!(published_vaults(&cache), 0);
        assert_eq!(
            schema_version(&cache).as_deref(),
            Some(super::SCHEMA_VERSION)
        );
    }

    #[test]
    fn a_previous_version_cache_from_another_model_is_wiped_not_migrated() {
        struct OtherModel(StubEmbedder);
        impl Embedder for OtherModel {
            fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
                self.0.embed(texts)
            }
            fn embedding_dim(&self) -> usize {
                384
            }
            fn identity(&self) -> String {
                "another-model-384".to_string()
            }
            fn token_count(&self, text: &str, add: bool) -> Result<usize, String> {
                self.0.token_count(text, add)
            }
        }

        let (_directory, path) = previous_version_cache();
        let cache = SqliteCache::open(&path, 384).expect("open");
        cache
            .reset_if_embedder_changed(&OtherModel(StubEmbedder::new(384)))
            .expect("a model change wipes");
        assert_eq!(published_vaults(&cache), 0);
        assert_eq!(
            schema_version(&cache).as_deref(),
            Some(super::SCHEMA_VERSION)
        );
    }
}
