//! The one cache migration ADR-50 allows: from the previous schema version to
//! the current one, keeping the vectors instead of re-embedding every chunk.
//! Delete this migration when the next schema change lands; a cache two
//! versions old is wiped and rebuilt, as before.
//!
//! Version 11 to 12 (#600, #308): the per-Vault vector tables filed each
//! vector under its Vault ID as TEXT metadata, which sqlite-vec leaks memory
//! on every write and delete of. Version 12 files them under an integer
//! `vault_key` from `vault_keys`. vec0 tables cannot be renamed
//! (asg017/sqlite-vec#125), so the rows are staged in plain tables, the vector
//! tables recreated, and the rows copied back. The unread `headings` and
//! `vault_headings` tables are dropped on the way.

use std::collections::BTreeMap;

use rusqlite::{Connection, Transaction, params};

use super::schema::{SCHEMA_VERSION, vault_vector_tables_sql};

pub(super) const PREVIOUS_SCHEMA_VERSION: &str = "11";

/// How many stored vectors per Vault and table must find themselves by KNN.
const SELF_RETRIEVAL_SAMPLE: usize = 16;

/// How many neighbours a self-retrieval query asks for first.
const FIRST_WINDOW: i64 = 16;

/// The largest `k` sqlite-vec accepts in a KNN query.
const KNN_K_MAX: i64 = 4096;

/// One per-Vault vector table and the plain table its rows wait in while the
/// vec0 table is recreated.
struct VectorTable {
    name: &'static str,
    stage: &'static str,
    /// Has the `layer` partition key, which a self-retrieval query must pin.
    partitioned: bool,
}

const VECTOR_TABLES: [VectorTable; 2] = [
    VectorTable {
        name: "vault_chunk_vectors",
        stage: "vault_vector_migration_default",
        partitioned: false,
    },
    VectorTable {
        name: "vault_chunk_vectors_demoted",
        stage: "vault_vector_migration_demoted",
        partitioned: true,
    },
];

/// Run the whole migration in one transaction and commit only after every
/// ADR-50 check has passed. The error names the step or check that failed.
/// On error the caller wipes the cache rather than trusting the rollback to
/// have restored the vec0 tables (asg017/sqlite-vec#333).
pub(super) fn migrate_from_previous_schema(
    conn: &mut Connection,
    embedding_dim: usize,
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|error| format!("start migration: {error}"))?;

    tx.execute_batch(
        "CREATE TABLE vault_keys (
             vault_key INTEGER PRIMARY KEY,
             vault_id TEXT NOT NULL UNIQUE
         );
         INSERT INTO vault_keys(vault_id)
             SELECT vault_id FROM vault_snapshots
             UNION SELECT vault_id FROM vault_chunks
             UNION SELECT vault_id FROM vault_chunk_vectors
             UNION SELECT vault_id FROM vault_chunk_vectors_demoted
             ORDER BY 1;",
    )
    .map_err(|error| format!("assign Vault keys: {error}"))?;

    let mut before = Vec::new();
    for table in &VECTOR_TABLES {
        before.push(rows_per_vault(&tx, table.name, "v.vault_id")?);
        stage_rows(&tx, table)?;
    }

    tx.execute_batch(
        "DROP TABLE vault_chunk_vectors;
         DROP TABLE vault_chunk_vectors_demoted;",
    )
    .map_err(|error| format!("drop the old vector tables: {error}"))?;
    tx.execute_batch(&vault_vector_tables_sql(embedding_dim))
        .map_err(|error| format!("create the new vector tables: {error}"))?;

    for (table, before) in VECTOR_TABLES.iter().zip(&before) {
        restore_rows(&tx, table)?;
        let after = rows_per_vault(
            &tx,
            table.name,
            "(SELECT vault_id FROM vault_keys k WHERE k.vault_key = v.vault_key)",
        )?;
        if &after != before {
            return Err(format!(
                "row count check failed on {}: {before:?} before, {after:?} after",
                table.name
            ));
        }
        check_identical(&tx, table, "after migrating")?;
        check_vault_of_chunk(&tx, table)?;
        check_self_retrieval(&tx, table)?;
    }

    tx.execute_batch(
        "DROP TABLE vault_vector_migration_default;
         DROP TABLE vault_vector_migration_demoted;
         DROP TABLE IF EXISTS vault_headings;
         DROP TABLE IF EXISTS headings;",
    )
    .map_err(|error| format!("drop the staged and retired tables: {error}"))?;

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

/// Vector rows per Vault, the Vault named by `vault_expr` over alias `v`.
fn rows_per_vault(
    tx: &Transaction<'_>,
    table: &str,
    vault_expr: &str,
) -> Result<BTreeMap<Option<String>, i64>, String> {
    let mut statement = tx
        .prepare(&format!(
            "SELECT {vault_expr}, COUNT(*) FROM {table} v GROUP BY 1"
        ))
        .map_err(|error| format!("count rows in {table}: {error}"))?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|error| format!("count rows in {table}: {error}"))?;
    rows.collect::<rusqlite::Result<_>>()
        .map_err(|error| format!("count rows in {table}: {error}"))
}

/// Copy every vector into a plain table under its Vault's key. The vector's
/// own Vault ID is mapped, not its chunk's, so a row filed under the wrong
/// Vault stays wrong and fails `check_vault_of_chunk` instead of being fixed
/// silently.
fn stage_rows(tx: &Transaction<'_>, table: &VectorTable) -> Result<(), String> {
    let layer = if table.partitioned {
        "v.layer AS layer,"
    } else {
        ""
    };
    tx.execute_batch(&format!(
        "CREATE TABLE {stage} AS
             SELECT v.chunk_id AS chunk_id, v.embedding AS embedding, {layer}
                    k.vault_key AS vault_key
             FROM {name} v LEFT JOIN vault_keys k ON k.vault_id = v.vault_id;
         CREATE UNIQUE INDEX {stage}_chunk ON {stage}(chunk_id);",
        stage = table.stage,
        name = table.name,
    ))
    .map_err(|error| format!("stage {}: {error}", table.name))?;
    // The staged copy must match the original byte for byte, so that the same
    // check after restoring compares the new table against the old one.
    check_identical(tx, table, "while staging")
}

fn restore_rows(tx: &Transaction<'_>, table: &VectorTable) -> Result<(), String> {
    let columns = if table.partitioned {
        "chunk_id, embedding, layer, vault_key"
    } else {
        "chunk_id, embedding, vault_key"
    };
    tx.execute_batch(&format!(
        "INSERT INTO {name}({columns}) SELECT {columns} FROM {stage} ORDER BY chunk_id;",
        name = table.name,
        stage = table.stage,
    ))
    .map_err(|error| format!("copy vectors back into {}: {error}", table.name))
}

/// Every row of the vector table must have its staged twin, with the same
/// chunk and the same bytes, and the two tables the same number of rows.
fn check_identical(tx: &Transaction<'_>, table: &VectorTable, phase: &str) -> Result<(), String> {
    let (staged, current, identical): (i64, i64, i64) = tx
        .query_row(
            &format!(
                "SELECT (SELECT COUNT(*) FROM {stage}),
                        (SELECT COUNT(*) FROM {name}),
                        (SELECT COUNT(*) FROM {name} v JOIN {stage} s
                             ON s.chunk_id = v.chunk_id AND s.embedding = v.embedding)",
                stage = table.stage,
                name = table.name,
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| format!("compare {} {phase}: {error}", table.name))?;
    if staged != current || current != identical {
        return Err(format!(
            "identical-vector check failed on {} {phase}: {identical} of {current} rows \
             match {staged} staged rows",
            table.name
        ));
    }
    Ok(())
}

fn check_vault_of_chunk(tx: &Transaction<'_>, table: &VectorTable) -> Result<(), String> {
    let (total, matched): (i64, i64) = tx
        .query_row(
            &format!(
                "SELECT (SELECT COUNT(*) FROM {name}),
                        (SELECT COUNT(*) FROM {name} v
                             JOIN vault_chunks c ON c.id = v.chunk_id
                             JOIN vault_keys k ON k.vault_key = v.vault_key
                             WHERE k.vault_id = c.vault_id)",
                name = table.name,
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("map {} to chunks: {error}", table.name))?;
    if total != matched {
        return Err(format!(
            "Vault mapping check failed on {}: {matched} of {total} rows belong to \
             their chunk's Vault",
            table.name
        ));
    }
    Ok(())
}

/// For every Vault, a sample of its stored vectors, spread over its rows,
/// must each come back from a KNN query for themselves, filtered to their
/// own Vault (and layer). Identical vectors tie at distance zero, so a vector
/// with many copies is looked for among all of them. Past sqlite-vec's `k`
/// limit it cannot ask for them all, and a full window of exact copies proves
/// the same thing.
fn check_self_retrieval(tx: &Transaction<'_>, table: &VectorTable) -> Result<(), String> {
    let rows = {
        let mut statement = tx
            .prepare(&format!(
                "SELECT vault_key, chunk_id FROM {} ORDER BY vault_key, chunk_id",
                table.stage
            ))
            .map_err(|error| format!("sample {}: {error}", table.name))?;
        statement
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
            .map_err(|error| format!("sample {}: {error}", table.name))?
    };
    let mut per_vault: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for (vault_key, chunk_id) in rows {
        per_vault.entry(vault_key).or_default().push(chunk_id);
    }

    let layer_filter = if table.partitioned {
        " AND layer = ?4"
    } else {
        ""
    };
    let knn = format!(
        "SELECT chunk_id, distance FROM {name} \
         WHERE embedding MATCH ?1 AND k = ?2 AND vault_key = ?3{layer_filter} \
         ORDER BY distance",
        name = table.name,
    );
    let layer_column = if table.partitioned { "layer" } else { "NULL" };
    let sample = format!(
        "SELECT embedding, {layer_column} FROM {stage} WHERE chunk_id = ?1",
        stage = table.stage,
    );
    let copies_of = format!(
        "SELECT COUNT(*) FROM {stage} WHERE embedding = ?1 AND vault_key = ?2",
        stage = table.stage,
    );
    for (vault_key, chunk_ids) in per_vault {
        let step = chunk_ids.len().div_ceil(SELF_RETRIEVAL_SAMPLE).max(1);
        for &chunk_id in chunk_ids.iter().step_by(step) {
            let (embedding, layer): (Vec<u8>, Option<String>) = tx
                .query_row(&sample, params![chunk_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .map_err(|error| format!("read sample vector from {}: {error}", table.name))?;
            let nearest = |k: i64| -> Result<Vec<(i64, f64)>, String> {
                let mut statement = tx.prepare(&knn).map_err(|error| {
                    format!("prepare self-retrieval on {}: {error}", table.name)
                })?;
                let hit =
                    |row: &rusqlite::Row<'_>| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?));
                if table.partitioned {
                    statement
                        .query_map(params![embedding, k, vault_key, layer], hit)
                        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
                } else {
                    statement
                        .query_map(params![embedding, k, vault_key], hit)
                        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
                }
                .map_err(|error| format!("run self-retrieval on {}: {error}", table.name))
            };
            let full_of_copies = |found: &[(i64, f64)], k: i64| {
                found.len() as i64 == k && found.iter().all(|&(_, distance)| distance == 0.0)
            };

            let found = nearest(FIRST_WINDOW)?;
            let mut found_itself = found.iter().any(|&(id, _)| id == chunk_id);
            // A window filled with exact copies hides the vector among its
            // twins, so only then count them and ask for every one.
            if !found_itself && full_of_copies(&found, FIRST_WINDOW) {
                let copies: i64 = tx
                    .query_row(&copies_of, params![embedding, vault_key], |row| row.get(0))
                    .map_err(|error| format!("count copies in {}: {error}", table.name))?;
                let k = copies.min(KNN_K_MAX);
                let found = nearest(k)?;
                found_itself = found.iter().any(|&(id, _)| id == chunk_id)
                    || (copies > KNN_K_MAX && full_of_copies(&found, k));
            }
            if !found_itself {
                return Err(format!(
                    "self-retrieval check failed on {}: chunk {chunk_id} is not its own \
                     nearest neighbour",
                    table.name
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rusqlite::params;
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

    /// The version 11 layout of everything this migration changes, verbatim.
    const PREVIOUS_LAYOUT: &str = "
        CREATE VIRTUAL TABLE vault_chunk_vectors USING vec0(
            chunk_id INTEGER PRIMARY KEY,
            embedding FLOAT[384],
            vault_id TEXT AUXILIARY
        );
        CREATE VIRTUAL TABLE vault_chunk_vectors_demoted USING vec0(
            chunk_id INTEGER PRIMARY KEY,
            embedding FLOAT[384],
            layer TEXT PARTITION KEY,
            vault_id TEXT AUXILIARY
        );
        CREATE TABLE headings (
            note_slug TEXT NOT NULL REFERENCES notes(slug) ON DELETE CASCADE ON UPDATE CASCADE,
            level INTEGER NOT NULL,
            text TEXT NOT NULL,
            anchor TEXT NOT NULL,
            position INTEGER NOT NULL,
            PRIMARY KEY (note_slug, anchor, position)
        );
        CREATE INDEX idx_headings_note_slug ON headings(note_slug);
        CREATE TABLE vault_headings (
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
        CREATE INDEX idx_vault_headings_note ON vault_headings(vault_id, note_slug);";

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
        ] {
            std::fs::write(root.join(path), format!("{body}\n\n{topic} notes")).expect("note");
        }
    }

    /// A cache file in the version 11 layout holding three published Vaults,
    /// with demoted vectors. Built by publishing with today's code, then
    /// rewriting the changed tables into the old layout with the old DDL.
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
                "CREATE TABLE old_default AS
                     SELECT v.chunk_id, v.embedding, k.vault_id
                     FROM vault_chunk_vectors v JOIN vault_keys k ON k.vault_key = v.vault_key;
                 CREATE TABLE old_demoted AS
                     SELECT v.chunk_id, v.embedding, v.layer, k.vault_id
                     FROM vault_chunk_vectors_demoted v
                     JOIN vault_keys k ON k.vault_key = v.vault_key;
                 DROP TABLE vault_chunk_vectors;
                 DROP TABLE vault_chunk_vectors_demoted;
                 DROP TABLE vault_keys;",
            )
            .expect("save the vectors");
            conn.execute_batch(PREVIOUS_LAYOUT).expect("old layout");
            conn.execute_batch(
                "INSERT INTO vault_chunk_vectors(chunk_id, embedding, vault_id)
                     SELECT chunk_id, embedding, vault_id FROM old_default;
                 INSERT INTO vault_chunk_vectors_demoted(chunk_id, embedding, layer, vault_id)
                     SELECT chunk_id, embedding, layer, vault_id FROM old_demoted;
                 INSERT INTO vault_headings(vault_id, note_slug, level, text, anchor, position)
                     SELECT vault_id, slug, 1, title, slug, 0 FROM vault_notes;
                 DROP TABLE old_default;
                 DROP TABLE old_demoted;
                 UPDATE metadata SET value = '11' WHERE key = 'schema_version';
                 UPDATE vault_snapshot_metadata SET value = '11'
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

    /// What version 11 answered: its KNN queries, filtering on the Vault ID
    /// text, merged the way the search code merged them.
    fn previous_version_search(
        cache: &SqliteCache,
        vault_ids: &[VaultId],
        query: &str,
        all_layers: bool,
    ) -> Vec<Hit> {
        let vector = StubEmbedder::new(384)
            .embed(&[query.to_string()])
            .expect("embed")
            .remove(0);
        let bytes: &[u8] = bytemuck::cast_slice(&vector);
        let ids = vault_ids
            .iter()
            .map(|id| format!("'{id}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let conn = cache.read().expect("read");
        let mut hits = Vec::new();
        let mut tables = vec!["vault_chunk_vectors"];
        if all_layers {
            tables.push("vault_chunk_vectors_demoted");
        }
        for table in tables {
            let mut statement = conn
                .prepare(&format!(
                    "SELECT c.vault_id, v.chunk_id, c.note_slug, v.distance \
                     FROM {table} v JOIN vault_chunks c ON c.id = v.chunk_id \
                     WHERE v.embedding MATCH ?1 AND v.k = ?2 AND v.vault_id IN ({ids}) \
                     ORDER BY v.distance"
                ))
                .expect("prepare");
            hits.extend(
                statement
                    .query_map(params![bytes, 5_i64], |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get::<_, f64>(3)? as f32,
                        ))
                    })
                    .expect("query")
                    .map(|row| row.expect("row")),
            );
        }
        hits.sort_by(|a: &Hit, b: &Hit| {
            a.3.total_cmp(&b.3)
                .then_with(|| a.0.cmp(&b.0))
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| a.1.cmp(&b.1))
        });
        hits.truncate(5);
        hits
    }

    fn current_search(
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

    /// Every scope and layer selection the fixed searches run under.
    fn fixed_searches(
        cache: &SqliteCache,
        search: fn(&SqliteCache, &[VaultId], &str, bool) -> Vec<Hit>,
    ) -> Vec<Vec<Hit>> {
        let ids = vault_ids();
        let mut scopes = vec![ids.clone()];
        scopes.extend(ids.iter().map(|id| vec![*id]));
        let mut results = Vec::new();
        for scope in &scopes {
            for query in QUERIES {
                for all_layers in [false, true] {
                    results.push(search(cache, scope, query, all_layers));
                }
            }
        }
        results
    }

    fn table_exists(cache: &SqliteCache, name: &str) -> bool {
        cache
            .connection()
            .expect("conn")
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
                [name],
                |row| row.get(0),
            )
            .expect("query")
    }

    fn published_vaults(cache: &SqliteCache) -> i64 {
        cache
            .connection()
            .expect("conn")
            .query_row("SELECT COUNT(*) FROM vault_snapshots", [], |row| row.get(0))
            .expect("count")
    }

    #[test]
    fn a_migrated_cache_answers_the_same_searches_without_re_embedding() {
        let (directory, path) = previous_version_cache();
        let cache = SqliteCache::open(&path, 384).expect("open the previous version");
        assert_eq!(
            cache
                .get_metadata("schema_version")
                .expect("version")
                .as_deref(),
            Some(super::PREVIOUS_SCHEMA_VERSION),
            "opening must leave the cache alone until the model check has run"
        );
        let before = fixed_searches(&cache, previous_version_search);
        assert!(before.iter().all(|hits| !hits.is_empty()));

        cache
            .prepare_for_embedder(&StubEmbedder::new(384))
            .expect("loading the model migrates the cache");
        cache
            .reset_if_embedder_changed(&StubEmbedder::new(384))
            .expect("the next Index turn's model check finds nothing to migrate");

        assert_eq!(
            cache
                .get_metadata("schema_version")
                .expect("version")
                .as_deref(),
            Some(super::SCHEMA_VERSION)
        );
        assert_eq!(published_vaults(&cache), 3, "migration must not wipe");
        assert_eq!(fixed_searches(&cache, current_search), before);
        assert!(!table_exists(&cache, "headings"));
        assert!(!table_exists(&cache, "vault_headings"));
        assert!(!table_exists(&cache, "vault_vector_migration_default"));

        // Each published snapshot still records the version 11 candidate it
        // came from. Seeding the next candidate from it must not make that
        // candidate look one version behind, or it wipes itself and the
        // Vault is re-embedded from scratch.
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
        assert!(!table_exists(&cache, "vault_chunk_vectors_metadatatext00"));
        assert!(!table_exists(
            &cache,
            "vault_chunk_vectors_demoted_metadatatext00"
        ));
    }

    #[test]
    fn a_failed_check_names_itself_and_commits_nothing() {
        let (_directory, path) = previous_version_cache();
        let cache = SqliteCache::open(&path, 384).expect("open");
        let mut conn = cache.connection().expect("conn");
        conn.execute(
            "INSERT INTO vault_chunk_vectors(chunk_id, embedding, vault_id) VALUES (?1, ?2, ?3)",
            params![
                999_999_i64,
                bytemuck::cast_slice::<f32, u8>(&[0.5_f32; 384]),
                VAULTS[0]
            ],
        )
        .expect("a vector with no chunk");

        let failure = super::migrate_from_previous_schema(&mut conn, 384)
            .expect_err("a vector with no chunk must fail the Vault mapping check");
        assert!(
            failure.contains("Vault mapping check failed on vault_chunk_vectors"),
            "{failure}"
        );
        let (version, old_layout, keys): (String, bool, bool) = conn
            .query_row(
                "SELECT (SELECT value FROM metadata WHERE key = 'schema_version'),
                        EXISTS(SELECT 1 FROM sqlite_master
                               WHERE name = 'vault_chunk_vectors_metadatatext00'),
                        EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'vault_keys')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read the cache after the failure");
        assert_eq!(
            version,
            super::PREVIOUS_SCHEMA_VERSION,
            "nothing may commit"
        );
        assert!(
            old_layout,
            "the old vector table must be back after the rollback"
        );
        assert!(!keys, "the new key table must be rolled back");
    }

    /// Identical vectors tie at distance zero, so a sampled vector can sit
    /// behind its twins in a KNN window, and past sqlite-vec's `k` limit of
    /// 4096 they cannot all be asked for.
    #[test]
    fn many_identical_vectors_still_pass_self_retrieval() {
        let (_directory, path) = previous_version_cache();
        let cache = SqliteCache::open(&path, 384).expect("open");
        let mut conn = cache.connection().expect("conn");
        let twin = bytemuck::cast_slice::<f32, u8>(&[0.25_f32; 384]).to_vec();
        for (vault_id, copies) in [(VAULTS[0], 4400_i64), (VAULTS[1], 40)] {
            conn.execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?2)
                 INSERT INTO vault_chunks(vault_id, note_slug, ordinal, content, byte_start,
                                          byte_end, content_hash)
                 SELECT ?1, (SELECT slug FROM vault_notes WHERE vault_id = ?1 LIMIT 1),
                        1000 + i, 'twin', 0, 4, 'twin'
                 FROM n",
                params![vault_id, copies],
            )
            .expect("chunks with identical text");
            conn.execute(
                "INSERT INTO vault_chunk_vectors(chunk_id, embedding, vault_id)
                 SELECT id, ?2, ?1 FROM vault_chunks WHERE vault_id = ?1 AND content = 'twin'",
                params![vault_id, twin],
            )
            .expect("identical vectors");
        }

        super::migrate_from_previous_schema(&mut conn, 384)
            .expect("identical vectors are no reason to wipe");
    }

    #[test]
    fn a_failed_migration_wipes_and_rebuilds_a_searchable_cache() {
        let (directory, path) = previous_version_cache();
        let cache = SqliteCache::open(&path, 384).expect("open");
        cache
            .connection()
            .expect("conn")
            .execute(
                "INSERT INTO vault_chunk_vectors(chunk_id, embedding, vault_id) VALUES (?1, ?2, ?3)",
                params![
                    999_999_i64,
                    bytemuck::cast_slice::<f32, u8>(&[0.5_f32; 384]),
                    VAULTS[0]
                ],
            )
            .expect("a vector with no chunk");

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
            cache
                .get_metadata("schema_version")
                .expect("version")
                .as_deref(),
            Some(super::SCHEMA_VERSION)
        );

        let vault_id = vault_ids()[0];
        let index = VaultIndex::build(directory.path().join("vault-0")).expect("index");
        cache
            .replace_vault_snapshot(vault_id, &index, &embedder)
            .expect("rebuild");
        assert!(!current_search(&cache, &[vault_id], "melatonin sleep", true).is_empty());
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
                    "UPDATE metadata SET value = '10' WHERE key = 'schema_version'",
                    [],
                )
                .expect("age the cache");
        }
        let cache = SqliteCache::open(&path, 384).expect("reopen");
        assert_eq!(published_vaults(&cache), 0);
        assert_eq!(
            cache
                .get_metadata("schema_version")
                .expect("version")
                .as_deref(),
            Some(super::SCHEMA_VERSION)
        );
        assert!(!table_exists(&cache, "headings"));
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
            cache
                .get_metadata("schema_version")
                .expect("version")
                .as_deref(),
            Some(super::SCHEMA_VERSION)
        );
        assert!(!table_exists(&cache, "vault_chunk_vectors_metadatatext00"));
    }
}
