use rusqlite::{Transaction, params};

use crate::chunk::Chunk;

#[allow(dead_code)]
pub struct ChunkRow<'a> {
    pub chunk: &'a Chunk,
    /// The chunk's embedding, or `None` when the note is not embedded: the chunk
    /// row is still written so keyword/FTS search works, but no vector is stored.
    pub vector: Option<&'a [f32]>,
}

/// Replace a note's chunk rows and their vectors.
///
/// `layer` routes the vector writes: `None` (the default surface) writes into
/// `chunk_vectors`, keeping default semantic search on its unfiltered KNN fast
/// path; a demoted layer writes into `chunk_vectors_demoted` with the layer as
/// the vec0 partition key, so a per-layer KNN is partition-pruned rather than
/// scanned. Vectors for this note are cleared from BOTH tables first so a note
/// that crossed a layer boundary since the last build cannot leave a stale
/// vector in the other table.
#[allow(dead_code)]
pub fn replace_chunks_for_note(
    tx: &Transaction<'_>,
    note_slug: &str,
    layer: Option<&str>,
    rows: &[ChunkRow<'_>],
    tags_json: Option<&str>,
    aliases_json: Option<&str>,
) -> Result<(), String> {
    tx.execute(
        "DELETE FROM chunk_vectors WHERE chunk_id IN (SELECT id FROM chunks WHERE note_slug = ?1)",
        params![note_slug],
    )
    .map_err(|e| format!("failed to clear chunk_vectors for {note_slug}: {e}"))?;
    tx.execute(
        "DELETE FROM chunk_vectors_demoted WHERE chunk_id IN (SELECT id FROM chunks WHERE note_slug = ?1)",
        params![note_slug],
    )
    .map_err(|e| format!("failed to clear chunk_vectors_demoted for {note_slug}: {e}"))?;
    tx.execute(
        "DELETE FROM chunks WHERE note_slug = ?1",
        params![note_slug],
    )
    .map_err(|e| format!("failed to clear chunks for {note_slug}: {e}"))?;

    if rows.is_empty() {
        return Ok(());
    }

    let mut insert_chunk = tx.prepare(
        r#"INSERT INTO chunks
           (note_slug, ordinal, heading_path, content, byte_start, byte_end, content_hash, tags, aliases)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
           RETURNING id"#,
    ).map_err(|e| format!("prepare chunk insert: {e}"))?;
    let mut insert_default_vector = tx
        .prepare("INSERT INTO chunk_vectors (chunk_id, embedding) VALUES (?1, ?2)")
        .map_err(|e| format!("prepare vector insert: {e}"))?;
    let mut insert_demoted_vector = tx
        .prepare(
            "INSERT INTO chunk_vectors_demoted (chunk_id, embedding, layer) VALUES (?1, ?2, ?3)",
        )
        .map_err(|e| format!("prepare demoted vector insert: {e}"))?;

    for row in rows {
        let chunk_id: i64 = insert_chunk
            .query_row(
                params![
                    note_slug,
                    row.chunk.ordinal as i64,
                    row.chunk.heading_path,
                    row.chunk.content,
                    row.chunk.byte_start as i64,
                    row.chunk.byte_end as i64,
                    row.chunk.content_hash,
                    tags_json,
                    aliases_json,
                ],
                |r| r.get(0),
            )
            .map_err(|e| format!("insert chunk: {e}"))?;
        if let Some(vector) = row.vector {
            let vector_bytes: &[u8] = bytemuck::cast_slice(vector);
            match layer {
                None => insert_default_vector
                    .execute(params![chunk_id, vector_bytes])
                    .map_err(|e| format!("insert vector: {e}"))?,
                Some(layer) => insert_demoted_vector
                    .execute(params![chunk_id, vector_bytes, layer])
                    .map_err(|e| format!("insert demoted vector: {e}"))?,
            };
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub fn existing_chunk_hashes(
    tx: &Transaction<'_>,
    note_slug: &str,
) -> Result<std::collections::HashMap<String, i64>, String> {
    let mut stmt = tx
        .prepare("SELECT content_hash, id FROM chunks WHERE note_slug = ?1")
        .map_err(|e| format!("prepare hash query: {e}"))?;
    let rows = stmt
        .query_map(params![note_slug], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| format!("query chunk hashes: {e}"))?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        let (hash, id) = row.map_err(|e| format!("read chunk hash row: {e}"))?;
        map.insert(hash, id);
    }
    Ok(map)
}

#[allow(dead_code)]
pub fn delete_orphan_vectors(tx: &Transaction<'_>) -> Result<usize, String> {
    let removed_default = tx
        .execute(
            "DELETE FROM chunk_vectors WHERE chunk_id NOT IN (SELECT id FROM chunks)",
            [],
        )
        .map_err(|e| format!("delete orphan vectors: {e}"))?;
    let removed_demoted = tx
        .execute(
            "DELETE FROM chunk_vectors_demoted WHERE chunk_id NOT IN (SELECT id FROM chunks)",
            [],
        )
        .map_err(|e| format!("delete orphan demoted vectors: {e}"))?;
    Ok(removed_default + removed_demoted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::SqliteCache;

    fn fake_chunk(ordinal: usize, content: &str) -> Chunk {
        Chunk {
            ordinal,
            heading_path: Some("H".to_string()),
            content: content.to_string(),
            byte_start: 0,
            byte_end: content.len(),
            content_hash: blake3::hash(content.as_bytes()).to_hex().to_string(),
        }
    }

    fn insert_minimal_note(cache: &SqliteCache, slug: &str) {
        let conn = cache.connection().expect("conn");
        conn.execute(
            r#"INSERT INTO notes (slug, title, normalized_title, relative_path,
                normalized_relative_path, absolute_path, content, content_hash,
                mtime_ns, size_bytes, indexed_at)
               VALUES (?, 'T', 't', ?, ?, '/abs', 'c', 'h', 0, 0, 0)"#,
            params![slug, format!("{slug}.md"), format!("{slug}.md")],
        )
        .expect("insert note");
    }

    #[test]
    fn replace_chunks_inserts_new_chunks_and_vectors() {
        let cache = SqliteCache::in_memory(384).expect("open");
        insert_minimal_note(&cache, "n1");
        let chunk = fake_chunk(0, "hello");
        let vector = vec![0.1f32; 384];

        {
            let mut conn = cache.connection().expect("conn");
            let tx = conn.transaction().expect("tx");
            replace_chunks_for_note(
                &tx,
                "n1",
                None,
                &[ChunkRow {
                    chunk: &chunk,
                    vector: Some(&vector),
                }],
                None,
                None,
            )
            .expect("replace");
            tx.commit().expect("commit");
        }

        let conn = cache.connection().expect("conn");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE note_slug = 'n1'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(count, 1);
        let vec_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunk_vectors", [], |r| r.get(0))
            .expect("count");
        assert_eq!(vec_count, 1);
    }

    #[test]
    fn replace_chunks_drops_previous_chunks_and_vectors_for_note() {
        let cache = SqliteCache::in_memory(384).expect("open");
        insert_minimal_note(&cache, "n1");
        let vector = vec![0.1f32; 384];

        {
            let mut conn = cache.connection().expect("conn");
            let tx = conn.transaction().expect("tx");
            replace_chunks_for_note(
                &tx,
                "n1",
                None,
                &[ChunkRow {
                    chunk: &fake_chunk(0, "old"),
                    vector: Some(&vector),
                }],
                None,
                None,
            )
            .expect("write");
            tx.commit().expect("commit");
        }

        {
            let mut conn = cache.connection().expect("conn");
            let tx = conn.transaction().expect("tx");
            replace_chunks_for_note(
                &tx,
                "n1",
                None,
                &[
                    ChunkRow {
                        chunk: &fake_chunk(0, "fresh-1"),
                        vector: Some(&vector),
                    },
                    ChunkRow {
                        chunk: &fake_chunk(1, "fresh-2"),
                        vector: Some(&vector),
                    },
                ],
                None,
                None,
            )
            .expect("rewrite");
            tx.commit().expect("commit");
        }

        let conn = cache.connection().expect("conn");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE note_slug = 'n1'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(count, 2);
        let vec_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunk_vectors", [], |r| r.get(0))
            .expect("count");
        assert_eq!(vec_count, 2);
    }

    #[test]
    fn existing_chunk_hashes_returns_hash_to_id_map() {
        let cache = SqliteCache::in_memory(384).expect("open");
        insert_minimal_note(&cache, "n1");
        let chunk = fake_chunk(0, "hello");
        let vector = vec![0.1f32; 384];

        let mut conn = cache.connection().expect("conn");
        let tx = conn.transaction().expect("tx");
        replace_chunks_for_note(
            &tx,
            "n1",
            None,
            &[ChunkRow {
                chunk: &chunk,
                vector: Some(&vector),
            }],
            None,
            None,
        )
        .expect("write");

        let map = existing_chunk_hashes(&tx, "n1").expect("read");
        assert_eq!(map.len(), 1);
        assert!(map.contains_key(&chunk.content_hash));
    }

    #[test]
    fn delete_orphan_vectors_removes_vectors_without_chunks() {
        let cache = SqliteCache::in_memory(384).expect("open");
        let mut conn = cache.connection().expect("conn");
        let tx = conn.transaction().expect("tx");
        let vec_bytes = bytemuck::cast_slice(&vec![0.1f32; 384]).to_vec();
        tx.execute(
            "INSERT INTO chunk_vectors (chunk_id, embedding) VALUES (?, ?)",
            params![9999i64, vec_bytes],
        )
        .expect("insert orphan");
        let removed = delete_orphan_vectors(&tx).expect("sweep");
        assert_eq!(removed, 1);
        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM chunk_vectors", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 0);
    }
}
