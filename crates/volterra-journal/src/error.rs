use thiserror::Error;

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("SQLite database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Job not found: {0}")]
    JobNotFound(String),

    #[error("File not found in job: id {0}")]
    FileNotFound(i64),

    #[error("Corrupt journal state: {0}")]
    CorruptState(String),

    #[error("Invalid schema version: expected {expected}, found {found}")]
    InvalidSchemaVersion { expected: u32, found: u32 },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
