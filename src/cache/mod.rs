mod chunk_ops;
pub(crate) mod parse;
mod populate;
mod queries;
mod schema;

use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, Once};

use rusqlite::Connection;

pub(crate) struct SqliteCache {
    pub(crate) conn: Mutex<Connection>,
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
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self, String> {
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
        cache.ensure_schema()?;
        Ok(cache)
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Result<Self, String> {
        register_sqlite_vec();
        let conn = Connection::open_in_memory()
            .map_err(|error| format!("failed to open in-memory SQLite cache: {error}"))?;
        let cache = Self {
            conn: Mutex::new(conn),
        };
        cache.ensure_schema()?;
        Ok(cache)
    }

    pub(crate) fn connection(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.conn
            .lock()
            .map_err(|_| "SQLite cache connection lock poisoned".to_string())
    }
}
