mod chunk_ops;
mod migrate;
pub mod parse;
mod populate;
mod queries;
mod schema;
pub(crate) mod vault_snapshots;

pub use populate::BuildOptions;
pub(crate) use populate::{BuildHandles, IndexYield};
pub use queries::SemanticHit;
use std::collections::BTreeMap;
use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, Once};

use rusqlite::Connection;
use rusqlite::OptionalExtension;

use crate::vault_registry::VaultId;

/// Where the cache database lives. File-backed caches get a pool of extra
/// read connections (WAL lets many readers run alongside the single writer);
/// in-memory caches share the one connection because each `:memory:` handle
/// would otherwise be an isolated empty database.
enum CacheSource {
    File(PathBuf),
    Memory,
}

/// Upper bound on active file-backed read connections, including checked-out
/// leases and connections retained idle for reuse.
const MAX_READ_CONNECTIONS: usize = 4;

/// How long a reader waits for a slot before giving up. [`MAX_READ_CONNECTIONS`]
/// bounds how many SQLite handles exist at once; it is not a load-shedding
/// policy, so a caller that would be served as soon as the request ahead of it
/// finishes waits rather than taking an immediate error. Failing fast there
/// turned an ordinary burst of UI requests into an outage. Matched to the
/// `busy_timeout` every connection already carries.
///
/// Waiters are woken one at a time and are not queued in arrival order, so a
/// thread can lose a freed slot to one arriving behind it. Reads hold a slot
/// for microseconds, which makes the unfairness cheap; a sustained overload
/// still resolves to this timeout rather than to a fair hand-off.
const READ_LEASE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

pub struct SqliteCache {
    /// The single writer connection. All mutations and transactions go through
    /// this; reads borrow it only for in-memory caches.
    pub conn: Mutex<Connection>,
    source: CacheSource,
    /// Idle read connections available for checkout (file-backed only).
    read_pool: Mutex<Vec<Connection>>,
    /// Monotonic per-Vault cache-publication attempts. This is disposable
    /// process state: it prevents an older failed or successful candidate from
    /// overwriting a newer attempt's cache status while the durable Markdown
    /// source remains untouched.
    #[allow(dead_code)]
    vault_snapshot_attempts: Mutex<BTreeMap<VaultId, u64>>,
    /// Serializes model identity reset, candidate construction, and shared
    /// snapshot publication so two Vault rebuilds cannot interleave models.
    pub(crate) snapshot_model_epoch: Mutex<()>,
    /// Active file-backed reader leases. Reserving before opening keeps bursts
    /// of concurrent reads from creating an unbounded number of SQLite handles.
    read_leases: Mutex<usize>,
    /// Signalled whenever a reader lease is returned, so a caller queued at the
    /// ceiling wakes as soon as a slot frees instead of polling.
    read_lease_available: Condvar,
    /// Reporting never takes database locks. The runtime waits for this signal
    /// and recovers off-thread after all callers have released their handles.
    corruption: tokio::sync::Notify,
    corruption_reported: AtomicBool,
    recovering: AtomicBool,
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
    query_deadline: Option<std::time::Instant>,
    reusable: bool,
}

/// A pinned SQLite read snapshot. Dropping an unfinished snapshot rolls it
/// back before its connection returns to the pool.
pub(crate) struct ReadSnapshot<'a> {
    conn: ReadConn<'a>,
    complete: bool,
}

impl Deref for ReadSnapshot<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.conn
    }
}

impl ReadSnapshot<'_> {
    pub(crate) fn limit_query_time(&mut self, duration: std::time::Duration) -> Result<(), String> {
        self.conn.limit_query_time(duration)
    }

    pub(crate) fn commit(&mut self) -> Result<(), String> {
        self.conn.cache.report_connection_error(&self.conn);
        self.conn.check_query_deadline()?;
        self.conn
            .execute_batch("COMMIT")
            .map_err(|error| format!("commit SQLite read snapshot: {error}"))?;
        self.complete = true;
        Ok(())
    }
}

impl Drop for ReadSnapshot<'_> {
    fn drop(&mut self) {
        self.conn.cache.report_connection_error(&self.conn);
        // Rollback must not itself be interrupted by the expired query.
        self.conn.clear_query_limit();
        if !self.complete {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
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

impl ReadConn<'_> {
    fn connection_mut(&mut self) -> &mut Connection {
        match (&mut self.pooled, &mut self.shared) {
            (Some(conn), _) => conn,
            (_, Some(guard)) => guard,
            (None, None) => unreachable!("ReadConn holds exactly one connection"),
        }
    }

    fn limit_query_time(&mut self, duration: std::time::Duration) -> Result<(), String> {
        let deadline = std::time::Instant::now() + duration;
        self.connection_mut()
            .progress_handler(1_000, Some(move || std::time::Instant::now() >= deadline))
            .map_err(|error| format!("install SQLite query time limit: {error}"))?;
        self.query_deadline = Some(deadline);
        Ok(())
    }

    fn check_query_deadline(&self) -> Result<(), String> {
        if self
            .query_deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            Err("keyword query exceeded its time limit; try fewer terms".to_string())
        } else {
            Ok(())
        }
    }

    fn clear_query_limit(&mut self) {
        if self.query_deadline.take().is_some() {
            // A connection cannot fail to remove its own handler while alive.
            // If SQLite ever refuses, discard a pooled handle rather than lend
            // an expired deadline to the next request.
            if let Err(error) = self
                .connection_mut()
                .progress_handler(0, None::<fn() -> bool>)
            {
                tracing::error!(%error, "could not clear SQLite query time limit");
                self.reusable = false;
            }
        }
    }
}

impl Drop for ReadConn<'_> {
    fn drop(&mut self) {
        self.cache.report_connection_error(self);
        self.clear_query_limit();
        if let Some(conn) = self.pooled.take() {
            if self.reusable {
                self.cache.return_read_connection(conn);
            } else {
                drop(conn);
                self.cache.release_read_lease();
            }
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
    pub(crate) async fn wait_for_corruption(&self) {
        self.corruption.notified().await;
    }

    /// String errors here originate in cache SQL, not filesystem or note
    /// parsing failures. Preserve the existing quarantine classification.
    pub(crate) fn report_corruption(&self, error: &str) {
        if is_physical_cache_corruption(error) {
            self.signal_corruption();
        }
    }

    fn signal_corruption(&self) {
        if matches!(self.source, CacheSource::File(_))
            && !self.recovering.load(Ordering::Acquire)
            && !self.corruption_reported.swap(true, Ordering::AcqRel)
        {
            tracing::error!("SQLite cache corruption detected; scheduling quarantine and rebuild");
            self.corruption.notify_one();
        }
    }

    fn report_connection_error(&self, conn: &Connection) {
        // SAFETY: the guard owns a live SQLite handle exclusively on this
        // thread. Reading its last error code neither mutates it nor retains
        // the pointer. Do this before COMMIT/ROLLBACK can clear that error.
        let code = unsafe { rusqlite::ffi::sqlite3_errcode(conn.handle()) };
        if matches!(
            code,
            rusqlite::ffi::SQLITE_CORRUPT | rusqlite::ffi::SQLITE_NOTADB
        ) {
            self.signal_corruption();
        }
    }

    /// Drain handles before renaming a WAL database. A reader left on the old
    /// inode must never return to the new pool, and an old writer must not
    /// checkpoint or unlink the replacement database's sidecars.
    pub(crate) fn recover_corruption(&self, embedding_dim: usize) -> Result<bool, String> {
        let CacheSource::File(path) = &self.source else {
            return Ok(false);
        };
        let _epoch = self
            .snapshot_model_epoch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.corruption_reported.load(Ordering::Acquire) {
            return Ok(false);
        }
        self.recovering.store(true, Ordering::Release);
        self.read_lease_available.notify_all();
        let leases = self
            .read_leases
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (leases, _) = self
            .read_lease_available
            .wait_timeout_while(leases, READ_LEASE_WAIT, |leases| *leases != 0)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *leases != 0 {
            return Err("cache recovery is waiting for active readers; retrying".into());
        }
        let mut writer = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.read_pool
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        let placeholder = Connection::open_in_memory()
            .map_err(|error| format!("close damaged SQLite writer: {error}"))?;
        drop(std::mem::replace(&mut *writer, placeholder));
        if path.exists() {
            let quarantine = quarantine_corrupt_cache(path)?;
            tracing::warn!(cache = %path.display(), quarantine = %quarantine.display(),
                "SQLite cache quarantined; rebuilding every active Vault from Markdown");
        }
        // On failure the recovering flag stays set: callers must not see the
        // empty placeholder, and the runtime retries instead of needing a restart.
        let replacement = Self::open_file(path, embedding_dim)?;
        *writer = replacement
            .conn
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.corruption_reported.store(false, Ordering::Release);
        self.recovering.store(false, Ordering::Release);
        drop(leases);
        self.read_lease_available.notify_all();
        Ok(true)
    }

    fn ensure_not_recovering(&self) -> Result<(), String> {
        if self.recovering.load(Ordering::Acquire) {
            Err("SQLite cache is being rebuilt after corruption; retry shortly".into())
        } else {
            Ok(())
        }
    }

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

        match Self::open_file(path, embedding_dim) {
            Ok(cache) => Ok(cache),
            Err(error) if is_physical_cache_corruption(&error) => {
                let quarantine = quarantine_corrupt_cache(path)?;
                tracing::warn!(
                    cache = %path.display(),
                    quarantine = %quarantine.display(),
                    error = %error,
                    "SQLite cache is physically malformed; quarantined and rebuilding from Markdown"
                );
                Self::open_file(path, embedding_dim)
            }
            Err(error) => Err(error),
        }
    }

    fn open_file(path: &Path, embedding_dim: usize) -> Result<Self, String> {
        register_sqlite_vec();
        let conn = Connection::open(path).map_err(|error| {
            format!("failed to open SQLite cache '{}': {error}", path.display())
        })?;
        check_physical_integrity(&conn)?;
        apply_writer_pragmas(&conn)?;
        let cache = Self {
            conn: Mutex::new(conn),
            source: CacheSource::File(path.to_path_buf()),
            corruption: tokio::sync::Notify::new(),
            corruption_reported: AtomicBool::new(false),
            recovering: AtomicBool::new(false),
            read_pool: Mutex::new(Vec::new()),
            vault_snapshot_attempts: Mutex::new(BTreeMap::new()),
            snapshot_model_epoch: Mutex::new(()),
            read_leases: Mutex::new(0),
            read_lease_available: Condvar::new(),
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
            corruption: tokio::sync::Notify::new(),
            corruption_reported: AtomicBool::new(false),
            recovering: AtomicBool::new(false),
            read_pool: Mutex::new(Vec::new()),
            vault_snapshot_attempts: Mutex::new(BTreeMap::new()),
            snapshot_model_epoch: Mutex::new(()),
            read_leases: Mutex::new(0),
            read_lease_available: Condvar::new(),
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
        self.ensure_not_recovering()?;
        let guard = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.ensure_not_recovering()?;
        self.report_connection_error(&guard);
        Ok(guard)
    }

    /// Check out a connection for read-only queries. File-backed caches draw
    /// from a pool of dedicated read connections so concurrent readers don't
    /// serialize behind the writer (WAL keeps them consistent). In-memory
    /// caches share the single writer connection. The returned guard returns
    /// its connection to the pool when dropped.
    pub fn read(&self) -> Result<ReadConn<'_>, String> {
        self.ensure_not_recovering()?;
        match &self.source {
            CacheSource::Memory => Ok(ReadConn {
                cache: self,
                pooled: None,
                shared: Some(self.connection()?),
                query_deadline: None,
                reusable: true,
            }),
            CacheSource::File(path) => {
                self.reserve_read_lease()?;
                let pooled = {
                    let mut pool = self
                        .read_pool
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    pool.pop()
                };
                let conn = match pooled {
                    Some(conn) => conn,
                    None => match open_read_connection(path) {
                        Ok(conn) => conn,
                        Err(error) => {
                            self.report_corruption(&error);
                            self.release_read_lease();
                            return Err(error);
                        }
                    },
                };
                Ok(ReadConn {
                    cache: self,
                    pooled: Some(conn),
                    shared: None,
                    query_deadline: None,
                    reusable: true,
                })
            }
        }
    }

    /// Pin one SQLite snapshot for related reads. This is for compound
    /// responses whose rows are assembled by more than one query: WAL readers
    /// otherwise see a fresh autocommit snapshot for each query and can combine
    /// revisions. Dropping without `commit` rolls the snapshot back.
    pub(crate) fn read_snapshot(&self) -> Result<ReadSnapshot<'_>, String> {
        let conn = self.read()?;
        conn.execute_batch("BEGIN")
            .map_err(|error| format!("begin SQLite read snapshot: {error}"))?;
        Ok(ReadSnapshot {
            conn,
            complete: false,
        })
    }

    fn return_read_connection(&self, conn: Connection) {
        // A connection still inside a transaction is closed rather than
        // pooled. A read snapshot whose COMMIT and ROLLBACK both failed would
        // otherwise pin one WAL generation on this connection, and every later
        // caller handed it would read that frozen generation forever.
        if conn.is_autocommit() {
            let mut pool = self
                .read_pool
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if pool.len() < MAX_READ_CONNECTIONS {
                pool.push(conn);
            }
        }
        // Otherwise the connection is dropped (closed) here. Either way, the
        // checked-out lease is available to the next caller.
        self.release_read_lease();
    }

    fn reserve_read_lease(&self) -> Result<(), String> {
        let leases = self
            .read_leases
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (mut leases, wait) = self
            .read_lease_available
            .wait_timeout_while(leases, READ_LEASE_WAIT, |leases| {
                *leases >= MAX_READ_CONNECTIONS && !self.recovering.load(Ordering::Acquire)
            })
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.ensure_not_recovering()?;
        if wait.timed_out() {
            return Err(format!(
                "SQLite read connection limit ({MAX_READ_CONNECTIONS}) reached; retry shortly"
            ));
        }
        *leases += 1;
        Ok(())
    }

    /// How many file-backed read leases are currently checked out.
    #[cfg(test)]
    pub(crate) fn active_read_leases(&self) -> usize {
        *self
            .read_leases
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn release_read_lease(&self) {
        {
            let mut leases = self
                .read_leases
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            debug_assert!(*leases > 0, "file-backed read lease underflow");
            *leases = leases.saturating_sub(1);
        }
        self.read_lease_available.notify_one();
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

fn check_physical_integrity(conn: &Connection) -> Result<(), String> {
    let result: String = conn
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(|error| format!("check SQLite cache integrity: {error}"))?;
    if result == "ok" {
        Ok(())
    } else {
        Err(format!("database corruption: {result}"))
    }
}

fn is_physical_cache_corruption(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("file is not a database")
        || normalized.contains("database disk image is malformed")
        || normalized.contains("database corruption")
}

fn quarantine_corrupt_cache(path: &Path) -> Result<PathBuf, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("failed creating cache quarantine timestamp: {error}"))?
        .as_nanos();
    let mut attempt = 0_u32;
    loop {
        let candidate = PathBuf::from(format!("{}.corrupt-{stamp}-{attempt}", path.display()));
        if !candidate.exists() {
            // No live handles remain. Preserve sidecars before moving the main
            // file, so an interrupted quarantine cannot replay an old WAL into
            // the replacement cache. A failed rename aborts before reopening.
            for suffix in ["-wal", "-shm", "-journal"] {
                let mut source = path.as_os_str().to_os_string();
                source.push(suffix);
                let mut destination = candidate.as_os_str().to_os_string();
                destination.push(suffix);
                match fs::rename(&source, &destination) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(format!("failed quarantining SQLite sidecar: {error}"));
                    }
                }
            }
            fs::rename(path, &candidate).map_err(|error| {
                format!(
                    "failed quarantining malformed SQLite cache '{}' as '{}': {error}",
                    path.display(),
                    candidate.display()
                )
            })?;
            return Ok(candidate);
        }
        attempt = attempt.saturating_add(1);
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Duration;

    use tempfile::tempdir;

    #[test]
    fn sqlite_time_limit_interrupts_cpu_work_and_is_removed_before_reuse() {
        let dir = tempdir().unwrap();
        let cache = SqliteCache::open(dir.path().join("cache.sqlite3"), 384).unwrap();
        let start = std::time::Instant::now();
        {
            let mut reader = cache.read().unwrap();
            reader.limit_query_time(Duration::from_millis(10)).unwrap();
            let error = reader.query_row(
                "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n",
                [], |row| row.get::<_, i64>(0),
            ).expect_err("SQLite CPU work must be interrupted");
            assert_eq!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::OperationInterrupted)
            );
        }
        assert!(start.elapsed() < Duration::from_secs(1));
        let reader = cache.read().unwrap();
        assert_eq!(
            reader.query_row(
                "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) SELECT sum(x) FROM n",
                [], |row| row.get::<_, i64>(0),
            ).unwrap(),
            50_005_000,
            "the reused handle must run beyond the old progress-handler interval"
        );
    }

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

    #[test]
    fn file_backed_read_leases_are_capped_and_recover_after_release() {
        let dir = tempdir().expect("temp dir");
        let cache =
            Arc::new(SqliteCache::open(dir.path().join("cache.sqlite3"), 384).expect("open"));
        let barrier = Arc::new(Barrier::new(MAX_READ_CONNECTIONS + 1));
        let (ready_tx, ready_rx) = mpsc::channel();
        let mut release_txs = Vec::new();
        let mut workers = Vec::new();

        for _ in 0..MAX_READ_CONNECTIONS {
            let cache = cache.clone();
            let barrier = barrier.clone();
            let ready_tx = ready_tx.clone();
            let (release_tx, release_rx) = mpsc::channel();
            release_txs.push(release_tx);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                let lease = cache.read();
                ready_tx.send(lease.is_ok()).expect("report lease");
                release_rx.recv().expect("release lease");
                drop(lease);
            }));
        }
        drop(ready_tx);

        barrier.wait();
        for _ in 0..MAX_READ_CONNECTIONS {
            assert!(ready_rx.recv().expect("lease result"));
        }

        // A caller arriving at the ceiling queues for the next free slot
        // rather than taking an immediate error; see READ_LEASE_WAIT.
        let queued_cache = cache.clone();
        let (queued_tx, queued_rx) = mpsc::channel();
        let queued = std::thread::spawn(move || {
            let lease = queued_cache.read();
            queued_tx.send(lease.is_ok()).expect("report queued lease");
        });
        assert!(
            matches!(
                queued_rx.recv_timeout(Duration::from_millis(200)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "a caller at the ceiling must wait for a slot, not be served or rejected"
        );
        assert_eq!(
            cache.active_read_leases(),
            MAX_READ_CONNECTIONS,
            "waiting must not push the handle count past the ceiling"
        );

        for release in release_txs {
            release.send(()).expect("release worker");
        }
        for worker in workers {
            worker.join().expect("reader worker");
        }
        assert!(
            queued_rx.recv().expect("queued lease result"),
            "a released lease must hand off to the caller queued behind it"
        );
        queued.join().expect("queued reader");

        cache.read().expect("a released lease must be reusable");
    }

    #[test]
    fn a_read_connection_left_inside_a_transaction_is_never_pooled() {
        let dir = tempdir().expect("temp dir");
        let cache = SqliteCache::open(dir.path().join("cache.sqlite3"), 384).expect("open");

        // Stands in for a snapshot whose COMMIT and ROLLBACK both failed: the
        // lease goes back with its read transaction still open.
        {
            let lease = cache.read().expect("lease");
            lease.execute_batch("BEGIN").expect("begin");
            assert!(!lease.is_autocommit());
        }

        let next = cache.read().expect("next lease");
        assert!(
            next.is_autocommit(),
            "a pooled connection must never carry an open transaction to its next caller"
        );
        drop(next);
        assert_eq!(cache.active_read_leases(), 0);
    }

    fn damage_three_data_pages(path: &Path) {
        let cache = SqliteCache::open(path, 384).expect("cache");
        {
            let conn = cache.connection().unwrap();
            conn.execute_batch(
                "CREATE TABLE damage_me(id INTEGER PRIMARY KEY, payload TEXT);
                WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<50)
                INSERT INTO damage_me SELECT x, printf('%3500s', 'x') FROM n;
                PRAGMA wal_checkpoint(TRUNCATE);",
            )
            .unwrap();
        }
        let (page_size, pages) = {
            let conn = cache.connection().unwrap();
            let page_size: i64 = conn
                .pragma_query_value(None, "page_size", |row| row.get(0))
                .unwrap();
            let mut statement = conn.prepare("SELECT pageno FROM dbstat WHERE name='damage_me' AND pagetype='leaf' ORDER BY pageno LIMIT 3").unwrap();
            let pages = statement
                .query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (page_size, pages)
        };
        assert_eq!(pages.len(), 3);
        drop(cache);
        overwrite_pages(path, page_size, pages);
    }

    fn overwrite_pages(path: &Path, page_size: i64, pages: Vec<i64>) {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        for page in pages {
            file.seek(SeekFrom::Start(((page - 1) * page_size) as u64))
                .unwrap();
            file.write_all(&vec![0x55; page_size as usize]).unwrap();
        }
        file.sync_all().unwrap();
    }

    #[test]
    fn transactional_writer_reports_corruption_even_when_rollback_clears_it() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cache.sqlite3");
        let cache = SqliteCache::open(&path, 384).unwrap();
        let vault_id: VaultId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let (size, pages) = {
            let conn = cache.connection().unwrap();
            conn.execute(
                "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<50)
                INSERT INTO vault_note_fts(vault_id,title,relative_path,content,slug)
                SELECT ?1,'Home','Home.md',printf('%3500s','x'),x FROM n",
                [vault_id.to_string()],
            )
            .unwrap();
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .unwrap();
            let size: i64 = conn
                .pragma_query_value(None, "page_size", |row| row.get(0))
                .unwrap();
            let pages = {
                let mut statement = conn.prepare("SELECT pageno FROM dbstat WHERE name='vault_note_fts_content' AND pagetype='leaf' ORDER BY pageno LIMIT 3").unwrap();
                statement
                    .query_map([], |row| row.get::<_, i64>(0))
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap()
            };
            conn.execute_batch("PRAGMA shrink_memory").unwrap();
            (size, pages)
        };
        assert_eq!(pages.len(), 3);
        overwrite_pages(&path, size, pages);
        let error = cache
            .disconnect_vault_snapshot(vault_id)
            .expect_err("damaged FTS data");
        assert!(error.contains("malformed"), "{error}");
        assert!(
            cache.recover_corruption(384).unwrap(),
            "a rollback must not lose the recovery signal"
        );
        cache
            .set_metadata("after_writer_recovery", "works")
            .unwrap();
        assert_eq!(
            cache
                .get_metadata("after_writer_recovery")
                .unwrap()
                .as_deref(),
            Some("works")
        );
    }

    #[test]
    fn runtime_corruption_replaces_connections_and_leaves_a_working_cache() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cache.sqlite3");
        let cache = SqliteCache::open(&path, 384).unwrap();
        damage_three_data_pages(&path);
        {
            let reader = cache.read().unwrap();
            let failure = reader
                .query_row("SELECT sum(length(payload)) FROM damage_me", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect_err("damaged leaf pages");
            assert_eq!(
                failure.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseCorrupt)
            );
        }
        assert!(cache.recover_corruption(384).expect("runtime recovery"));
        cache
            .set_metadata("after_runtime_recovery", "works")
            .unwrap();
        let reader = cache.read().unwrap();
        assert_eq!(
            reader
                .query_row(
                    "SELECT value FROM metadata WHERE key='after_runtime_recovery'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "works"
        );
        assert!(
            std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
        );
    }

    #[test]
    fn damaged_data_pages_are_quarantined_even_when_schema_is_readable() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cache.sqlite3");
        damage_three_data_pages(&path);
        let cache = SqliteCache::open(&path, 384).expect("rebuild damaged data pages");
        assert!(cache.get_metadata("schema_version").unwrap().is_some());
        cache.set_metadata("after_rebuild", "works").unwrap();
        assert_eq!(
            cache.get_metadata("after_rebuild").unwrap().as_deref(),
            Some("works")
        );
        assert!(
            std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
        );
    }

    #[test]
    fn malformed_disposable_cache_file_is_quarantined_and_rebuilt() {
        let dir = tempdir().expect("temp dir");
        let path = dir.path().join("cache.sqlite3");
        std::fs::write(&path, b"this is not an SQLite database").expect("write malformed cache");

        let cache = SqliteCache::open(&path, 384)
            .expect("a malformed disposable cache must be quarantined and rebuilt");
        cache
            .get_metadata("schema_version")
            .expect("rebuilt cache must be queryable")
            .expect("rebuilt cache must carry schema metadata");

        let quarantined = std::fs::read_dir(dir.path())
            .expect("read cache directory")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .any(|name| name.starts_with("cache.sqlite3.corrupt-"));
        assert!(
            quarantined,
            "the malformed bytes must be quarantined, not overwritten"
        );
    }
}
