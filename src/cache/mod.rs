mod chunk_ops;
pub mod parse;
mod populate;
mod queries;
mod schema;

pub use queries::SemanticHit;

use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, Once};

use rusqlite::Connection;
use rusqlite::OptionalExtension;

pub struct SqliteCache {
    pub conn: Mutex<Connection>,
}

static SQLITE_VEC_INIT: Once = Once::new();

fn register_sqlite_vec() {
    SQLITE_VEC_INIT.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut i8,
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
        let cache = Self {
            conn: Mutex::new(conn),
        };
        cache.ensure_schema(embedding_dim)?;
        Ok(cache)
    }

    pub fn in_memory(embedding_dim: usize) -> Result<Self, String> {
        register_sqlite_vec();
        let conn = Connection::open_in_memory()
            .map_err(|error| format!("failed to open in-memory SQLite cache: {error}"))?;
        let cache = Self {
            conn: Mutex::new(conn),
        };
        cache.ensure_schema(embedding_dim)?;
        Ok(cache)
    }

    #[cfg(test)]
    pub fn in_memory_with_dim(embedding_dim: usize) -> Result<Self, String> {
        Self::in_memory(embedding_dim)
    }

    pub fn connection(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.conn
            .lock()
            .map_err(|_| "SQLite cache connection lock poisoned".to_string())
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
}

#[cfg(test)]
mod metadata_tests {
    use super::*;

    #[test]
    fn set_and_get_metadata_roundtrip() {
        let cache = SqliteCache::in_memory(384).expect("open");
        cache.set_metadata("embedder_id", "BGESmallENV15").expect("set");
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
        assert_eq!(cache.get_metadata("k").expect("get").as_deref(), Some("second"));
    }
}
