//! Query history.
//!
//! One row per executed query: `(profile, sql, started_at, duration, result)`.
//! The table is cheap to keep forever but the frontend caps list queries to
//! a recent window. A future pruner can trim rows older than N days; for now
//! we assume the SQL text is small and let the table grow.
//!
//! Stored alongside the MRU db under `%APPDATA%\pg-shell\history.sqlite`
//! rather than cohabiting so either can be wiped independently.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("unable to resolve app data directory")]
    NoAppDataDir,
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub profile_id: String,
    pub sql: String,
    /// Unix epoch seconds.
    pub started_at: i64,
    /// Null while the query is still running; set on completion.
    pub duration_ms: Option<i64>,
    pub row_count: Option<i64>,
    pub cancelled: bool,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct HistoryStore {
    conn: Arc<Mutex<Connection>>,
}

impl HistoryStore {
    pub fn open_default() -> Result<Self, HistoryError> {
        let dirs =
            ProjectDirs::from("dev", "pg-shell", "pg-shell").ok_or(HistoryError::NoAppDataDir)?;
        let dir = dirs.config_dir().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Self::open_at(dir.join("history.sqlite"))
    }

    pub fn open_at(path: PathBuf) -> Result<Self, HistoryError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                profile_id TEXT NOT NULL,
                sql TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                duration_ms INTEGER,
                row_count INTEGER,
                cancelled INTEGER NOT NULL DEFAULT 0,
                error TEXT
             );
             CREATE INDEX IF NOT EXISTS history_profile_time
                ON history (profile_id, started_at DESC);",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Insert a "started" row and return its id. The caller updates the
    /// same row with `complete` / `error` once the query lifecycle ends.
    pub fn record_started(&self, profile_id: &str, sql: &str) -> Result<i64, HistoryError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO history (profile_id, sql, started_at) VALUES (?1, ?2, ?3)",
            params![profile_id, sql, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn record_complete(
        &self,
        id: i64,
        duration_ms: i64,
        row_count: i64,
        cancelled: bool,
    ) -> Result<(), HistoryError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE history SET duration_ms = ?2, row_count = ?3, cancelled = ?4 WHERE id = ?1",
            params![id, duration_ms, row_count, cancelled as i64],
        )?;
        Ok(())
    }

    pub fn record_error(&self, id: i64, message: &str) -> Result<(), HistoryError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE history SET error = ?2 WHERE id = ?1",
            params![id, message],
        )?;
        Ok(())
    }

    /// List most-recent-first, optionally filtered by a case-insensitive
    /// substring match on the SQL text.
    pub fn list(
        &self,
        profile_id: &str,
        limit: i64,
        search: Option<&str>,
    ) -> Result<Vec<HistoryEntry>, HistoryError> {
        let conn = self.conn.lock();
        let mut rows: Vec<HistoryEntry> = Vec::new();
        match search.filter(|s| !s.is_empty()) {
            Some(q) => {
                let like = format!("%{}%", q);
                let mut stmt = conn.prepare(
                    "SELECT id, profile_id, sql, started_at, duration_ms, row_count, cancelled, error
                       FROM history
                      WHERE profile_id = ?1 AND sql LIKE ?2 COLLATE NOCASE
                      ORDER BY started_at DESC, id DESC
                      LIMIT ?3",
                )?;
                let iter = stmt.query_map(params![profile_id, like, limit], map_row)?;
                for r in iter {
                    rows.push(r?);
                }
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT id, profile_id, sql, started_at, duration_ms, row_count, cancelled, error
                       FROM history
                      WHERE profile_id = ?1
                      ORDER BY started_at DESC, id DESC
                      LIMIT ?2",
                )?;
                let iter = stmt.query_map(params![profile_id, limit], map_row)?;
                for r in iter {
                    rows.push(r?);
                }
            }
        }
        Ok(rows)
    }

    pub fn clear(&self, profile_id: &str) -> Result<u64, HistoryError> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "DELETE FROM history WHERE profile_id = ?1",
            params![profile_id],
        )?;
        Ok(n as u64)
    }
}

fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: r.get(0)?,
        profile_id: r.get(1)?,
        sql: r.get(2)?,
        started_at: r.get(3)?,
        duration_ms: r.get(4)?,
        row_count: r.get(5)?,
        cancelled: r.get::<_, i64>(6)? != 0,
        error: r.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn record_and_list() {
        let dir = tempdir().unwrap();
        let store = HistoryStore::open_at(dir.path().join("history.sqlite")).unwrap();
        let id = store.record_started("p1", "SELECT 1").unwrap();
        store.record_complete(id, 5, 1, false).unwrap();

        let list = store.list("p1", 10, None).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].duration_ms, Some(5));
        assert_eq!(list[0].row_count, Some(1));
    }

    #[test]
    fn search_filters() {
        let dir = tempdir().unwrap();
        let store = HistoryStore::open_at(dir.path().join("history.sqlite")).unwrap();
        store.record_started("p1", "SELECT users").unwrap();
        store.record_started("p1", "SELECT orders").unwrap();
        let found = store.list("p1", 10, Some("users")).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].sql.contains("users"));
    }
}
