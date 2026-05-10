mod parse;
mod populate;
mod queries;
mod schema;

use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;

pub(crate) struct SqliteCache {
    pub(crate) conn: Mutex<Connection>,
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

        let conn = Connection::open(path).map_err(|error| {
            format!("failed to open SQLite cache '{}': {error}", path.display())
        })?;
        let cache = Self {
            conn: Mutex::new(conn),
        };
        cache.ensure_schema()?;
        Ok(cache)
    }

    pub(crate) fn in_memory() -> Result<Self, String> {
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
