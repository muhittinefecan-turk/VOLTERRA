use crate::error::JournalError;
use crate::models::CheckpointMode;
use crate::schema::apply_schema;
use parking_lot::{Mutex, MutexGuard};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

pub struct JournalEngine {
    pub(crate) conn: Mutex<Connection>,
    pub(crate) db_path: Option<PathBuf>,
}

impl JournalEngine {
    /// Opens or creates SQLite database with WAL mode, foreign keys, and performance pragmas.
    pub fn open(db_path: &Path) -> Result<Self, JournalError> {
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let conn = Connection::open(db_path)?;
        apply_schema(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path: Some(db_path.to_path_buf()),
        })
    }

    /// In-memory database for unit testing without disk I/O.
    pub fn open_in_memory() -> Result<Self, JournalError> {
        let conn = Connection::open_in_memory()?;
        apply_schema(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path: None,
        })
    }

    /// Obtains a mutex lock on the SQLite connection.
    pub fn lock_conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock()
    }

    /// Returns the database path, if not an in-memory database.
    pub fn db_path(&self) -> Option<&Path> {
        self.db_path.as_deref()
    }

    /// Executes WAL checkpoint with the specified mode.
    /// Returns `(busy, log, checkpointed)` counts as `(busy_or_log, checkpointed_frames)`.
    pub fn checkpoint(&self, mode: CheckpointMode) -> Result<(i32, i32), JournalError> {
        let mode_str = match mode {
            CheckpointMode::Passive => "PASSIVE",
            CheckpointMode::Full => "FULL",
            CheckpointMode::Restart => "RESTART",
            CheckpointMode::Truncate => "TRUNCATE",
        };

        let conn = self.conn.lock();
        let query = format!("PRAGMA wal_checkpoint({});", mode_str);
        let mut stmt = conn.prepare(&query)?;
        let result: (i32, i32, i32) =
            stmt.query_row([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;

        tracing::debug!(
            mode = mode_str,
            busy = result.0,
            log = result.1,
            checkpointed = result.2,
            "WAL checkpoint executed"
        );

        Ok((result.1, result.2))
    }
}
