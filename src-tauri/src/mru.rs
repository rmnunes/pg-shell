//! Most-recently-used tracking for completion items.
//!
//! SQLite-backed counter keyed on `(profile_id, kind, identifier)`. Every
//! time the user accepts a completion we record it; when completions are
//! produced we read the counts back and boost the score of previously-used
//! items.
//!
//! Scoring: `log2(1 + accept_count) * 10`. Saturates gracefully so a
//! once-used item gets +10, a hundred-times-used gets +66. That comfortably
//! fits inside the engine's score ranges without dominating context weight.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MruError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("unable to resolve app data directory")]
    NoAppDataDir,
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
}

/// Cloneable handle. `Arc<Mutex<Connection>>` is cheap on the hot path — every
/// write is a single indexed upsert and reads are a single profile-scoped
/// scan that we keep short.
#[derive(Clone)]
pub struct MruStore {
    conn: Arc<Mutex<Connection>>,
}

impl MruStore {
    pub fn open_default() -> Result<Self, MruError> {
        let dirs = ProjectDirs::from("dev", "pg-shell", "pg-shell")
            .ok_or(MruError::NoAppDataDir)?;
        let dir = dirs.config_dir().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Self::open_at(dir.join("mru.sqlite"))
    }

    pub fn open_at(path: PathBuf) -> Result<Self, MruError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS mru (
                profile_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                identifier TEXT NOT NULL,
                accept_count INTEGER NOT NULL DEFAULT 1,
                last_accepted INTEGER NOT NULL,
                PRIMARY KEY (profile_id, kind, identifier)
             );
             CREATE INDEX IF NOT EXISTS mru_recent
                ON mru (profile_id, last_accepted);",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn record(
        &self,
        profile_id: &str,
        kind: &str,
        identifier: &str,
    ) -> Result<(), MruError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO mru (profile_id, kind, identifier, accept_count, last_accepted)
             VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT(profile_id, kind, identifier) DO UPDATE SET
                accept_count = accept_count + 1,
                last_accepted = excluded.last_accepted",
            params![profile_id, kind, identifier, now],
        )?;
        Ok(())
    }

    /// All accept counts for a profile, keyed by `(kind, identifier)`. The
    /// map is small (bounded by what the user has actually accepted) so we
    /// materialize it eagerly and keep lookups inline for the scoring pass.
    pub fn counts_for(
        &self,
        profile_id: &str,
    ) -> Result<HashMap<(String, String), i64>, MruError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT kind, identifier, accept_count FROM mru WHERE profile_id = ?1",
        )?;
        let rows = stmt.query_map(params![profile_id], |r| {
            Ok((
                (r.get::<_, String>(0)?, r.get::<_, String>(1)?),
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (k, v) = row?;
            out.insert(k, v);
        }
        Ok(out)
    }

    /// Boost formula: `log2(1 + count) * 10`, floored to i32.
    pub fn boost(count: i64) -> i32 {
        if count <= 0 {
            return 0;
        }
        let x = (count + 1) as f64;
        (x.log2() * 10.0).floor() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn record_increments_count() {
        let dir = tempdir().unwrap();
        let store = MruStore::open_at(dir.path().join("mru.sqlite")).unwrap();
        store.record("p1", "table", "users").unwrap();
        store.record("p1", "table", "users").unwrap();
        store.record("p1", "table", "users").unwrap();
        let counts = store.counts_for("p1").unwrap();
        assert_eq!(counts.get(&("table".into(), "users".into())), Some(&3));
    }

    #[test]
    fn boost_monotone_saturating() {
        assert_eq!(MruStore::boost(0), 0);
        assert!(MruStore::boost(1) < MruStore::boost(10));
        assert!(MruStore::boost(10) < MruStore::boost(100));
        assert!(MruStore::boost(100) < 100);
    }
}
