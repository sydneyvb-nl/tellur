//! SQLite implementation of [`Store`] — the default single-node backend.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::Store;

/// Current schema version. Bumped as migrations are added in later phases.
const SCHEMA_VERSION: &str = "0";

/// A SQLite-backed store. The connection is behind a `Mutex` so the store is
/// `Send + Sync` and usable as `Arc<dyn Store>`.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open (or create) a database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database at {}", path.display()))?;
        Self::init(conn)
    }

    /// Open an ephemeral in-memory database (tests).
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().context("failed to open in-memory database")?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // Pragmas for durability + concurrency suitable for a server.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database connection lock poisoned"))
    }
}

impl Store for SqliteStore {
    fn migrate(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )
        .context("failed to create schema_meta")?;
        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [SCHEMA_VERSION],
        )
        .context("failed to record schema version")?;
        Ok(())
    }

    fn health_check(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .context("database health check failed")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_is_idempotent_and_records_version() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.migrate().unwrap();
        store.migrate().unwrap(); // idempotent

        let conn = store.conn().unwrap();
        let version: String = conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn health_check_passes_after_open() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert!(store.health_check().is_ok());
    }
}
