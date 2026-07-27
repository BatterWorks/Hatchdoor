mod chunk_ops;
pub mod parse;
mod populate;
mod queries;
mod schema;

pub use populate::BuildOptions;
pub use queries::SemanticHit;

use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, Once};

use rusqlite::Connection;
use rusqlite::OptionalExtension;

/// Where the cache database lives. File-backed caches get a pool of extra
/// read connections (WAL lets many readers run alongside the single writer);
/// in-memory caches share the one connection because each `:memory:` handle
/// would otherwise be an isolated empty database.
enum CacheSource {
    File(PathBuf),
    Memory,
}

/// Upper bound on pooled read connections kept idle for a file-backed cache.
const MAX_READ_CONNECTIONS: usize = 4;

pub struct SqliteCache {
    /// The single writer connection. All mutations and transactions go through
    /// this; reads borrow it only for in-memory caches.
    pub conn: Mutex<Connection>,
    source: CacheSource,
    /// Idle read connections available for checkout (file-backed only).
    read_pool: Mutex<Vec<Connection>>,
}

static SQLITE_VEC_INIT: Once = Once::new();

/// Pragmas applied to every connection (writer and readers).
fn apply_common_pragmas(conn: &Connection) -> Result<(), String> {
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("failed to set busy_timeout: {e}"))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| format!("failed to enable foreign_keys: {e}"))?;
    Ok(())
}

/// Writer-connection pragmas: WAL plus the common settings. WAL lets readers
/// on other connections run concurrently with the single writer.
fn apply_writer_pragmas(conn: &Connection) -> Result<(), String> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("failed to enable WAL: {e}"))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| format!("failed to set synchronous: {e}"))?;
    apply_common_pragmas(conn)
}

/// Open a fresh read connection to a file-backed cache. Guarded as `query_only`
/// so a stray write can't corrupt the writer's WAL state.
fn open_read_connection(path: &Path) -> Result<Connection, String> {
    register_sqlite_vec();
    let conn = Connection::open(path).map_err(|e| {
        format!(
            "failed to open SQLite read connection '{}': {e}",
            path.display()
        )
    })?;
    apply_common_pragmas(&conn)?;
    conn.pragma_update(None, "query_only", "ON")
        .map_err(|e| format!("failed to set query_only: {e}"))?;
    Ok(conn)
}

/// A connection checked out for read-only queries. Derefs to [`Connection`] so
/// existing query code is unchanged. On drop it returns a pooled connection to
/// the pool; the shared in-memory guard is simply released.
pub struct ReadConn<'a> {
    cache: &'a SqliteCache,
    pooled: Option<Connection>,
    shared: Option<MutexGuard<'a, Connection>>,
}

impl Deref for ReadConn<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        match (&self.pooled, &self.shared) {
            (Some(conn), _) => conn,
            (_, Some(guard)) => guard,
            (None, None) => unreachable!("ReadConn holds exactly one connection"),
        }
    }
}

impl Drop for ReadConn<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.pooled.take() {
            self.cache.return_read_connection(conn);
        }
    }
}

fn register_sqlite_vec() {
    SQLITE_VEC_INIT.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> i32,
        >(
            sqlite_vec::sqlite3_vec_init as *const ()
        )));
    });
}

impl SqliteCache {
    pub fn open(path: impl AsRef<Path>, embedding_dim: usize) -> Result<Self, String> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create SQLite cache directory '{}': {error}",
                    parent.display()
                )
            })?;
        }

        register_sqlite_vec();
        let conn = Connection::open(path).map_err(|error| {
            format!("failed to open SQLite cache '{}': {error}", path.display())
        })?;
        apply_writer_pragmas(&conn)?;
        let cache = Self {
            conn: Mutex::new(conn),
            source: CacheSource::File(path.to_path_buf()),
            read_pool: Mutex::new(Vec::new()),
        };
        cache.ensure_schema(embedding_dim)?;
        Ok(cache)
    }

    pub fn in_memory(embedding_dim: usize) -> Result<Self, String> {
        register_sqlite_vec();
        let conn = Connection::open_in_memory()
            .map_err(|error| format!("failed to open in-memory SQLite cache: {error}"))?;
        apply_common_pragmas(&conn)?;
        let cache = Self {
            conn: Mutex::new(conn),
            source: CacheSource::Memory,
            read_pool: Mutex::new(Vec::new()),
        };
        cache.ensure_schema(embedding_dim)?;
        Ok(cache)
    }

    #[cfg(test)]
    pub fn in_memory_with_dim(embedding_dim: usize) -> Result<Self, String> {
        Self::in_memory(embedding_dim)
    }

    pub fn connection(&self) -> Result<MutexGuard<'_, Connection>, String> {
        // A panic while the lock was held poisons the Mutex, but the SQLite
        // connection itself stays consistent: a rusqlite Transaction rolls back
        // on unwind (RAII), so the worst a panic leaves behind is a rolled-back
        // write. Recover the guard rather than erroring, or a single panic would
        // permanently wedge every future reindex and cache write.
        Ok(self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()))
    }

    /// Check out a connection for read-only queries. File-backed caches draw
    /// from a pool of dedicated read connections so concurrent readers don't
    /// serialize behind the writer (WAL keeps them consistent). In-memory
    /// caches share the single writer connection. The returned guard returns
    /// its connection to the pool when dropped.
    pub fn read(&self) -> Result<ReadConn<'_>, String> {
        match &self.source {
            CacheSource::Memory => Ok(ReadConn {
                cache: self,
                pooled: None,
                shared: Some(self.connection()?),
            }),
            CacheSource::File(path) => {
                let pooled = {
                    let mut pool = self
                        .read_pool
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    pool.pop()
                };
                let conn = match pooled {
                    Some(conn) => conn,
                    None => open_read_connection(path)?,
                };
                Ok(ReadConn {
                    cache: self,
                    pooled: Some(conn),
                    shared: None,
                })
            }
        }
    }

    fn return_read_connection(&self, conn: Connection) {
        if let Ok(mut pool) = self.read_pool.lock()
            && pool.len() < MAX_READ_CONNECTIONS
        {
            pool.push(conn);
        }
        // Otherwise the connection is dropped (closed) here.
    }

    pub fn set_metadata(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO metadata(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )
        .map_err(|e| format!("set_metadata({key}): {e}"))?;
        Ok(())
    }

    pub fn get_metadata(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.connection()?;
        let v = conn
            .query_row(
                "SELECT value FROM metadata WHERE key = ?1",
                rusqlite::params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("get_metadata({key}): {e}"))?;
        Ok(v)
    }

    /// The vault's layers (name + optional description), as persisted at the last
    /// populate. Drives the MCP `layers` enum and its per-value docs, which are
    /// built at request time when the in-memory `LayerMap` is no longer around.
    /// A vault with no markers (or a cache from before this key was written)
    /// returns an empty list, so the MCP surface simply advertises no layers.
    pub fn layer_catalog(&self) -> Result<Vec<crate::search::LayerInfo>, String> {
        match self.get_metadata("layer_catalog")? {
            Some(json) => serde_json::from_str(&json)
                .map_err(|e| format!("failed parsing persisted layer_catalog: {e}")),
            None => Ok(Vec::new()),
        }
    }

    /// Note counts grouped by layer (`None` = the default surface), reflecting
    /// the last populate. Drives the diagnostics surface's per-layer tally and
    /// its vanished-marker detection (a layer with notes but no live marker).
    pub fn layer_note_counts(&self) -> Result<Vec<(Option<String>, i64)>, String> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare("SELECT layer, COUNT(*) FROM notes GROUP BY layer ORDER BY layer IS NOT NULL, layer")
            .map_err(|e| format!("prepare layer_note_counts: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| format!("query layer_note_counts: {e}"))?;
        let mut counts = Vec::new();
        for row in rows {
            counts.push(row.map_err(|e| format!("row layer_note_counts: {e}"))?);
        }
        Ok(counts)
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;

    #[test]
    fn set_and_get_metadata_roundtrip() {
        let cache = SqliteCache::in_memory(384).expect("open");
        cache
            .set_metadata("embedder_id", "BGESmallENV15")
            .expect("set");
        let v = cache.get_metadata("embedder_id").expect("get");
        assert_eq!(v.as_deref(), Some("BGESmallENV15"));
    }

    #[test]
    fn get_metadata_returns_none_for_missing_key() {
        let cache = SqliteCache::in_memory(384).expect("open");
        let v = cache.get_metadata("does_not_exist").expect("get");
        assert!(v.is_none());
    }

    #[test]
    fn set_metadata_overwrites_existing_value() {
        let cache = SqliteCache::in_memory(384).expect("open");
        cache.set_metadata("k", "first").expect("set 1");
        cache.set_metadata("k", "second").expect("set 2");
        assert_eq!(
            cache.get_metadata("k").expect("get").as_deref(),
            Some("second")
        );
    }

    #[test]
    fn writer_lock_recovers_after_a_panicking_holder() {
        use std::sync::Arc;

        // A panic while the writer lock is held poisons the Mutex. That must not
        // permanently wedge the cache: every later reindex/write would otherwise
        // fail for the rest of the process lifetime.
        let cache = Arc::new(SqliteCache::in_memory(384).expect("open"));
        let poisoner = cache.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.connection().expect("lock");
            panic!("boom while holding the writer lock");
        })
        .join();

        // The connection is still usable despite the poison.
        cache
            .set_metadata("after_poison", "ok")
            .expect("cache must recover from a poisoned writer lock");
        assert_eq!(
            cache.get_metadata("after_poison").expect("get").as_deref(),
            Some("ok")
        );
    }
}
