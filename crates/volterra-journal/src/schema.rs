use crate::error::JournalError;
use rusqlite::Connection;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

pub const PRAGMA_SETUP: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA cache_size = -64000;
PRAGMA temp_store = MEMORY;
PRAGMA mmap_size = 268435456;
"#;

pub const SCHEMA_V1_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    source_root TEXT NOT NULL,
    target_root TEXT NOT NULL,
    source_volume_uuid TEXT,
    target_volume_uuid TEXT,
    mode TEXT NOT NULL CHECK (mode IN ('Turbo', 'SmartHybrid', 'SafeRecovery')),
    state TEXT NOT NULL CHECK (state IN ('Pending', 'Scanning', 'Running', 'Paused', 'Completed', 'Failed', 'WaitingForDisk', 'Cancelled')),
    total_files INTEGER NOT NULL DEFAULT 0,
    total_bytes INTEGER NOT NULL DEFAULT 0,
    copied_bytes INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_jobs_state ON jobs(state);

CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    source_size INTEGER NOT NULL,
    source_mtime_ns INTEGER NOT NULL,
    source_btime_ns INTEGER,
    posix_mode INTEGER NOT NULL,
    source_file_id TEXT NOT NULL,
    temp_path TEXT,
    state TEXT NOT NULL CHECK (state IN ('Pending', 'Scanning', 'Copying', 'Verifying', 'Completed', 'Paused', 'Skipped', 'ReadError', 'WriteError', 'Partial', 'NeedsAttention', 'WaitingForDisk', 'Cancelled')),
    verified_digest TEXT,
    error_code INTEGER,
    recovered_bytes INTEGER NOT NULL DEFAULT 0,
    UNIQUE(job_id, relative_path)
);

CREATE INDEX IF NOT EXISTS idx_files_job_state ON files(job_id, state);
CREATE INDEX IF NOT EXISTS idx_files_job_path ON files(job_id, relative_path);

CREATE TABLE IF NOT EXISTS chunks (
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    offset_bytes INTEGER NOT NULL,
    length_bytes INTEGER NOT NULL,
    digest TEXT,
    state TEXT NOT NULL CHECK (state IN ('Pending', 'Copying', 'Verified', 'Failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(file_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_chunks_file_state ON chunks(file_id, state);
CREATE INDEX IF NOT EXISTS idx_chunks_file_index ON chunks(file_id, chunk_index);

CREATE TABLE IF NOT EXISTS errors (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    file_id INTEGER REFERENCES files(id) ON DELETE CASCADE,
    offset_bytes INTEGER NOT NULL,
    length_bytes INTEGER NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN ('scan', 'read', 'write', 'verify', 'publish')),
    errno_code INTEGER NOT NULL,
    error_type TEXT NOT NULL,
    message TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 1,
    action_taken TEXT,
    recorded_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_errors_job ON errors(job_id);
CREATE INDEX IF NOT EXISTS idx_errors_file ON errors(file_id);
CREATE INDEX IF NOT EXISTS idx_errors_recorded ON errors(recorded_at);
"#;

pub fn configure_pragmas(conn: &Connection) -> Result<(), JournalError> {
    conn.execute_batch(PRAGMA_SETUP)?;
    Ok(())
}

pub fn apply_schema(conn: &Connection) -> Result<(), JournalError> {
    configure_pragmas(conn)?;

    conn.execute_batch(SCHEMA_V1_DDL)?;

    let mut stmt = conn.prepare("SELECT MAX(version) FROM schema_migrations")?;
    let current_version: Option<u32> = stmt.query_row([], |row| row.get(0)).unwrap_or(None);

    if current_version.is_none() {
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
            rusqlite::params![CURRENT_SCHEMA_VERSION, now],
        )?;
    }

    Ok(())
}
