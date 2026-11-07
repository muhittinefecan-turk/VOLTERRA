use crate::db::JournalEngine;
use crate::error::JournalError;
use crate::models::{
    file_state_as_str, file_state_from_str, ChunkRecord, ChunkState, ExtentErrorRecord, FileRecord,
    FileState, JobMode, JobRecord, JobRecoveryState, JobState,
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use std::path::PathBuf;

impl JournalEngine {
    /// Creates a new job record in the database.
    pub fn create_job(&self, job: &JobRecord) -> Result<(), JournalError> {
        let conn = self.conn.lock();
        conn.execute(
            r#"INSERT INTO jobs (
                id, source_root, target_root, source_volume_uuid, target_volume_uuid,
                mode, state, total_files, total_bytes, copied_bytes,
                created_at, updated_at, completed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"#,
            params![
                job.id,
                job.source_root.to_string_lossy(),
                job.target_root.to_string_lossy(),
                job.source_volume_uuid,
                job.target_volume_uuid,
                job.mode.as_str(),
                job.state.as_str(),
                job.total_files,
                job.total_bytes,
                job.copied_bytes,
                job.created_at,
                job.updated_at,
                job.completed_at,
            ],
        )?;
        Ok(())
    }

    /// Retrieves a job by ID.
    pub fn get_job(&self, job_id: &str) -> Result<Option<JobRecord>, JournalError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"SELECT id, source_root, target_root, source_volume_uuid, target_volume_uuid,
                      mode, state, total_files, total_bytes, copied_bytes,
                      created_at, updated_at, completed_at
               FROM jobs WHERE id = ?1"#,
        )?;

        let job = stmt
            .query_row(params![job_id], |row| {
                let mode_str: String = row.get(5)?;
                let state_str: String = row.get(6)?;
                Ok(JobRecord {
                    id: row.get(0)?,
                    source_root: PathBuf::from(row.get::<_, String>(1)?),
                    target_root: PathBuf::from(row.get::<_, String>(2)?),
                    source_volume_uuid: row.get(3)?,
                    target_volume_uuid: row.get(4)?,
                    mode: mode_str.parse().unwrap_or(JobMode::SmartHybrid),
                    state: state_str.parse().unwrap_or(JobState::Pending),
                    total_files: row.get(7)?,
                    total_bytes: row.get(8)?,
                    copied_bytes: row.get(9)?,
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                    completed_at: row.get(12)?,
                })
            })
            .optional()?;

        Ok(job)
    }

    /// Lists all jobs ordered by created_at DESC.
    pub fn list_jobs(&self) -> Result<Vec<JobRecord>, JournalError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"SELECT id, source_root, target_root, source_volume_uuid, target_volume_uuid,
                      mode, state, total_files, total_bytes, copied_bytes,
                      created_at, updated_at, completed_at
               FROM jobs ORDER BY created_at DESC"#,
        )?;

        let rows = stmt.query_map([], |row| {
            let mode_str: String = row.get(5)?;
            let state_str: String = row.get(6)?;
            Ok(JobRecord {
                id: row.get(0)?,
                source_root: PathBuf::from(row.get::<_, String>(1)?),
                target_root: PathBuf::from(row.get::<_, String>(2)?),
                source_volume_uuid: row.get(3)?,
                target_volume_uuid: row.get(4)?,
                mode: mode_str.parse().unwrap_or(JobMode::SmartHybrid),
                state: state_str.parse().unwrap_or(JobState::Pending),
                total_files: row.get(7)?,
                total_bytes: row.get(8)?,
                copied_bytes: row.get(9)?,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
                completed_at: row.get(12)?,
            })
        })?;

        let mut jobs = Vec::new();
        for job in rows {
            jobs.push(job?);
        }
        Ok(jobs)
    }

    /// Updates job lifecycle status.
    pub fn update_job_status(&self, job_id: &str, status: JobState) -> Result<(), JournalError> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().to_rfc3339();
        let completed_at = if status == JobState::Completed {
            Some(now.clone())
        } else {
            None
        };

        let rows = conn.execute(
            r#"UPDATE jobs SET state = ?1, updated_at = ?2,
                      completed_at = COALESCE(?3, completed_at)
               WHERE id = ?4"#,
            params![status.as_str(), now, completed_at, job_id],
        )?;

        if rows == 0 {
            return Err(JournalError::JobNotFound(job_id.to_string()));
        }
        Ok(())
    }

    /// Updates job state and copied bytes atomically.
    pub fn update_job_state(
        &self,
        job_id: &str,
        state: JobState,
        copied_bytes: u64,
    ) -> Result<(), JournalError> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().to_rfc3339();
        let completed_at = if state == JobState::Completed {
            Some(now.clone())
        } else {
            None
        };

        let rows = conn.execute(
            r#"UPDATE jobs SET state = ?1, copied_bytes = ?2, updated_at = ?3,
                      completed_at = COALESCE(?4, completed_at)
               WHERE id = ?5"#,
            params![state.as_str(), copied_bytes, now, completed_at, job_id],
        )?;

        if rows == 0 {
            return Err(JournalError::JobNotFound(job_id.to_string()));
        }
        Ok(())
    }

    /// Updates job progress (copied bytes) only.
    pub fn update_job_progress(&self, job_id: &str, copied_bytes: u64) -> Result<(), JournalError> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().to_rfc3339();
        let rows = conn.execute(
            r#"UPDATE jobs SET copied_bytes = ?1, updated_at = ?2 WHERE id = ?3"#,
            params![copied_bytes, now, job_id],
        )?;
        if rows == 0 {
            return Err(JournalError::JobNotFound(job_id.to_string()));
        }
        Ok(())
    }

    /// Atomically records a batch of files in a single transaction, updating job totals.
    pub fn record_files_batch(
        &self,
        job_id: &str,
        files: &[FileRecord],
    ) -> Result<(), JournalError> {
        if files.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let mut batch_bytes: u64 = 0;
        {
            let mut stmt = tx.prepare_cached(
                r#"INSERT INTO files (
                    job_id, relative_path, source_size, source_mtime_ns, source_btime_ns,
                    posix_mode, source_file_id, temp_path, state, verified_digest,
                    error_code, recovered_bytes
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
            )?;

            for file in files {
                batch_bytes = batch_bytes.saturating_add(file.source_size);
                stmt.execute(params![
                    job_id,
                    file.relative_path.to_string_lossy(),
                    file.source_size,
                    file.source_mtime_ns,
                    file.source_btime_ns,
                    file.posix_mode,
                    file.source_file_id,
                    file.temp_path.as_ref().map(|p| p.to_string_lossy()),
                    file_state_as_str(&file.state),
                    file.verified_digest,
                    file.error_code,
                    file.recovered_bytes,
                ])?;
            }
        }

        let files_count = files.len() as u64;
        let now = chrono::Utc::now().to_rfc3339();
        tx.execute(
            r#"UPDATE jobs SET
                total_files = total_files + ?1,
                total_bytes = total_bytes + ?2,
                updated_at = ?3
               WHERE id = ?4"#,
            params![files_count, batch_bytes, now, job_id],
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Records a verified chunk and automatically increments file and job recovered/copied bytes.
    pub fn record_chunk_verified(&self, chunk: &ChunkRecord) -> Result<(), JournalError> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute(
            r#"INSERT INTO chunks (file_id, chunk_index, offset_bytes, length_bytes, digest, state, attempts)
               VALUES (?1, ?2, ?3, ?4, ?5, 'Verified', ?6)
               ON CONFLICT(file_id, chunk_index) DO UPDATE SET
                   digest = excluded.digest,
                   state = 'Verified',
                   attempts = excluded.attempts"#,
            params![
                chunk.file_id,
                chunk.chunk_index,
                chunk.offset_bytes,
                chunk.length_bytes,
                chunk.digest,
                chunk.attempts,
            ],
        )?;

        // Update recovered_bytes for this file
        tx.execute(
            r#"UPDATE files SET recovered_bytes = (
                SELECT COALESCE(SUM(length_bytes), 0) FROM chunks WHERE file_id = ?1 AND state = 'Verified'
            ) WHERE id = ?1"#,
            params![chunk.file_id],
        )?;

        // Update copied_bytes for the associated job
        let now = chrono::Utc::now().to_rfc3339();
        tx.execute(
            r#"UPDATE jobs SET
                copied_bytes = (
                    SELECT COALESCE(SUM(c.length_bytes), 0)
                    FROM chunks c
                    JOIN files f ON c.file_id = f.id
                    WHERE f.job_id = (SELECT job_id FROM files WHERE id = ?1) AND c.state = 'Verified'
                ),
                updated_at = ?2
               WHERE id = (SELECT job_id FROM files WHERE id = ?1)"#,
            params![chunk.file_id, now],
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Records a batch of chunks in an atomic transaction.
    pub fn record_chunks_batch(&self, chunks: &[ChunkRecord]) -> Result<(), JournalError> {
        if chunks.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        {
            let mut stmt = tx.prepare_cached(
                r#"INSERT INTO chunks (file_id, chunk_index, offset_bytes, length_bytes, digest, state, attempts)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                   ON CONFLICT(file_id, chunk_index) DO UPDATE SET
                       digest = excluded.digest,
                       state = excluded.state,
                       attempts = excluded.attempts"#,
            )?;

            for chunk in chunks {
                stmt.execute(params![
                    chunk.file_id,
                    chunk.chunk_index,
                    chunk.offset_bytes,
                    chunk.length_bytes,
                    chunk.digest,
                    chunk.state.as_str(),
                    chunk.attempts,
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Retrieves all verified chunks for a given file.
    pub fn get_verified_chunks(&self, file_id: i64) -> Result<Vec<ChunkRecord>, JournalError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"SELECT file_id, chunk_index, offset_bytes, length_bytes, digest, state, attempts
               FROM chunks
               WHERE file_id = ?1 AND state = 'Verified'
               ORDER BY chunk_index ASC"#,
        )?;

        let rows = stmt.query_map(params![file_id], |row| {
            let state_str: String = row.get(5)?;
            Ok(ChunkRecord {
                file_id: row.get(0)?,
                chunk_index: row.get(1)?,
                offset_bytes: row.get(2)?,
                length_bytes: row.get(3)?,
                digest: row.get(4)?,
                state: state_str.parse().unwrap_or(ChunkState::Verified),
                attempts: row.get(6)?,
            })
        })?;

        let mut chunks = Vec::new();
        for chunk in rows {
            chunks.push(chunk?);
        }
        Ok(chunks)
    }

    /// Retrieves all unverified chunks for a given file.
    pub fn get_unverified_chunks(&self, file_id: i64) -> Result<Vec<ChunkRecord>, JournalError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"SELECT file_id, chunk_index, offset_bytes, length_bytes, digest, state, attempts
               FROM chunks
               WHERE file_id = ?1 AND state != 'Verified'
               ORDER BY chunk_index ASC"#,
        )?;

        let rows = stmt.query_map(params![file_id], |row| {
            let state_str: String = row.get(5)?;
            Ok(ChunkRecord {
                file_id: row.get(0)?,
                chunk_index: row.get(1)?,
                offset_bytes: row.get(2)?,
                length_bytes: row.get(3)?,
                digest: row.get(4)?,
                state: state_str.parse().unwrap_or(ChunkState::Pending),
                attempts: row.get(6)?,
            })
        })?;

        let mut chunks = Vec::new();
        for chunk in rows {
            chunks.push(chunk?);
        }
        Ok(chunks)
    }

    /// Updates file lifecycle status.
    pub fn update_file_status(&self, file_id: i64, status: FileState) -> Result<(), JournalError> {
        let conn = self.conn.lock();
        let rows = conn.execute(
            r#"UPDATE files SET state = ?1 WHERE id = ?2"#,
            params![file_state_as_str(&status), file_id],
        )?;

        if rows == 0 {
            return Err(JournalError::FileNotFound(file_id));
        }
        Ok(())
    }

    /// Updates file upon completion with final BLAKE3 digest and verified bytes.
    pub fn update_file_completion(
        &self,
        file_id: i64,
        digest: &str,
        recovered_bytes: u64,
    ) -> Result<(), JournalError> {
        let conn = self.conn.lock();
        let rows = conn.execute(
            r#"UPDATE files SET
                state = 'Completed',
                verified_digest = ?1,
                recovered_bytes = ?2
               WHERE id = ?3"#,
            params![digest, recovered_bytes, file_id],
        )?;

        if rows == 0 {
            return Err(JournalError::FileNotFound(file_id));
        }
        Ok(())
    }

    /// Records a logical extent error and returns the inserted error record ID.
    pub fn record_extent_error(&self, error: &ExtentErrorRecord) -> Result<i64, JournalError> {
        let conn = self.conn.lock();
        let now = if error.recorded_at.is_empty() {
            chrono::Utc::now().to_rfc3339()
        } else {
            error.recorded_at.clone()
        };

        conn.execute(
            r#"INSERT INTO errors (
                job_id, file_id, offset_bytes, length_bytes, phase,
                errno_code, error_type, message, attempts, action_taken, recorded_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
            params![
                error.job_id,
                error.file_id,
                error.offset_bytes,
                error.length_bytes,
                error.phase,
                error.errno_code,
                error.error_type,
                error.message,
                error.attempts,
                error.action_taken,
                now,
            ],
        )?;

        let id = conn.last_insert_rowid();
        Ok(id)
    }

    /// Retrieves all extent errors for a job.
    pub fn get_job_errors(&self, job_id: &str) -> Result<Vec<ExtentErrorRecord>, JournalError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"SELECT id, job_id, file_id, offset_bytes, length_bytes, phase,
                      errno_code, error_type, attempts, message, action_taken, recorded_at
               FROM errors WHERE job_id = ?1 ORDER BY recorded_at ASC"#,
        )?;

        let rows = stmt.query_map(params![job_id], |row| {
            Ok(ExtentErrorRecord {
                id: Some(row.get(0)?),
                job_id: row.get(1)?,
                file_id: row.get(2)?,
                offset_bytes: row.get(3)?,
                length_bytes: row.get(4)?,
                phase: row.get(5)?,
                errno_code: row.get(6)?,
                error_type: row.get(7)?,
                attempts: row.get(8)?,
                message: row.get(9)?,
                action_taken: row.get(10)?,
                recorded_at: row.get(11)?,
            })
        })?;

        let mut errors = Vec::new();
        for err in rows {
            errors.push(err?);
        }
        Ok(errors)
    }

    /// Retrieves all extent errors for a specific file.
    pub fn get_file_errors(&self, file_id: i64) -> Result<Vec<ExtentErrorRecord>, JournalError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"SELECT id, job_id, file_id, offset_bytes, length_bytes, phase,
                      errno_code, error_type, attempts, message, action_taken, recorded_at
               FROM errors WHERE file_id = ?1 ORDER BY recorded_at ASC"#,
        )?;

        let rows = stmt.query_map(params![file_id], |row| {
            Ok(ExtentErrorRecord {
                id: Some(row.get(0)?),
                job_id: row.get(1)?,
                file_id: row.get(2)?,
                offset_bytes: row.get(3)?,
                length_bytes: row.get(4)?,
                phase: row.get(5)?,
                errno_code: row.get(6)?,
                error_type: row.get(7)?,
                attempts: row.get(8)?,
                message: row.get(9)?,
                action_taken: row.get(10)?,
                recorded_at: row.get(11)?,
            })
        })?;

        let mut errors = Vec::new();
        for err in rows {
            errors.push(err?);
        }
        Ok(errors)
    }

    /// Updates remediation action taken on an error record.
    pub fn update_error_action(&self, error_id: i64, action: &str) -> Result<(), JournalError> {
        let conn = self.conn.lock();
        conn.execute(
            r#"UPDATE errors SET action_taken = ?1 WHERE id = ?2"#,
            params![action, error_id],
        )?;
        Ok(())
    }

    /// Alias for update_error_action conforming to trait definition.
    pub fn update_extent_error_action(
        &self,
        error_id: i64,
        action: &str,
    ) -> Result<(), JournalError> {
        self.update_error_action(error_id, action)
    }

    /// Bulk marks all unresolved errors for a job as 'Skipped' and updates corresponding files to Skipped.
    pub fn skip_all_errors(&self, job_id: &str) -> Result<usize, JournalError> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let rows_updated = tx.execute(
            r#"UPDATE errors SET action_taken = 'Skipped'
               WHERE job_id = ?1 AND (action_taken IS NULL OR action_taken = '' OR action_taken = 'Logged')"#,
            params![job_id],
        )?;

        // Update corresponding files whose state was ReadError or NeedsAttention to Skipped
        tx.execute(
            r#"UPDATE files SET state = 'Skipped'
               WHERE job_id = ?1 AND state IN ('ReadError', 'WriteError', 'NeedsAttention')"#,
            params![job_id],
        )?;

        tx.commit()?;
        Ok(rows_updated)
    }

    /// Loads full recovery state for an interrupted job.
    pub fn load_job_state(&self, job_id: &str) -> Result<JobRecoveryState, JournalError> {
        let job = self
            .get_job(job_id)?
            .ok_or_else(|| JournalError::JobNotFound(job_id.to_string()))?;

        let conn = self.conn.lock();

        // Count completed files
        let completed_count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM files WHERE job_id = ?1 AND state = 'Completed'",
            params![job_id],
            |row| row.get(0),
        )?;

        // Count failed files
        let failed_count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM files WHERE job_id = ?1 AND state IN ('ReadError', 'WriteError', 'Failed')",
            params![job_id],
            |row| row.get(0),
        )?;

        // Count pending files
        let pending_count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM files WHERE job_id = ?1 AND state = 'Pending'",
            params![job_id],
            |row| row.get(0),
        )?;

        // Interrupted files (Copying or Verifying)
        let mut stmt = conn.prepare(
            r#"SELECT id, job_id, relative_path, source_size, source_mtime_ns, source_btime_ns,
                      posix_mode, source_file_id, temp_path, state, verified_digest,
                      error_code, recovered_bytes
               FROM files
               WHERE job_id = ?1 AND state IN ('Copying', 'Verifying')"#,
        )?;

        let rows = stmt.query_map(params![job_id], |row| {
            let state_str: String = row.get(9)?;
            let temp_str: Option<String> = row.get(8)?;
            Ok(FileRecord {
                id: row.get(0)?,
                job_id: row.get(1)?,
                relative_path: PathBuf::from(row.get::<_, String>(2)?),
                source_size: row.get(3)?,
                source_mtime_ns: row.get(4)?,
                source_btime_ns: row.get(5)?,
                posix_mode: row.get(6)?,
                source_file_id: row.get(7)?,
                temp_path: temp_str.map(PathBuf::from),
                state: file_state_from_str(&state_str).unwrap_or(FileState::Copying),
                verified_digest: row.get(10)?,
                error_code: row.get(11)?,
                recovered_bytes: row.get(12)?,
            })
        })?;

        let mut interrupted_files = Vec::new();
        for file in rows {
            interrupted_files.push(file?);
        }

        // Open errors
        let mut err_stmt = conn.prepare(
            r#"SELECT id, job_id, file_id, offset_bytes, length_bytes, phase,
                      errno_code, error_type, attempts, message, action_taken, recorded_at
               FROM errors
               WHERE job_id = ?1 AND (action_taken IS NULL OR action_taken = 'Logged')
               ORDER BY recorded_at ASC"#,
        )?;

        let err_rows = err_stmt.query_map(params![job_id], |row| {
            Ok(ExtentErrorRecord {
                id: Some(row.get(0)?),
                job_id: row.get(1)?,
                file_id: row.get(2)?,
                offset_bytes: row.get(3)?,
                length_bytes: row.get(4)?,
                phase: row.get(5)?,
                errno_code: row.get(6)?,
                error_type: row.get(7)?,
                attempts: row.get(8)?,
                message: row.get(9)?,
                action_taken: row.get(10)?,
                recorded_at: row.get(11)?,
            })
        })?;

        let mut open_errors = Vec::new();
        for err in err_rows {
            open_errors.push(err?);
        }

        let total_copied_bytes = job.copied_bytes;

        Ok(JobRecoveryState {
            job,
            interrupted_files,
            completed_file_count: completed_count,
            failed_file_count: failed_count,
            pending_file_count: pending_count,
            total_copied_bytes,
            open_errors,
        })
    }

    /// Exports job report in JSON format.
    pub fn export_job_report_json(&self, job_id: &str) -> Result<String, JournalError> {
        let job = self
            .get_job(job_id)?
            .ok_or_else(|| JournalError::JobNotFound(job_id.to_string()))?;

        let errors = self.get_job_errors(job_id)?;

        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"SELECT id, job_id, relative_path, source_size, source_mtime_ns, source_btime_ns,
                      posix_mode, source_file_id, temp_path, state, verified_digest,
                      error_code, recovered_bytes
               FROM files WHERE job_id = ?1 ORDER BY id ASC"#,
        )?;

        let rows = stmt.query_map(params![job_id], |row| {
            let state_str: String = row.get(9)?;
            let temp_str: Option<String> = row.get(8)?;
            Ok(FileRecord {
                id: row.get(0)?,
                job_id: row.get(1)?,
                relative_path: PathBuf::from(row.get::<_, String>(2)?),
                source_size: row.get(3)?,
                source_mtime_ns: row.get(4)?,
                source_btime_ns: row.get(5)?,
                posix_mode: row.get(6)?,
                source_file_id: row.get(7)?,
                temp_path: temp_str.map(PathBuf::from),
                state: file_state_from_str(&state_str).unwrap_or(FileState::Pending),
                verified_digest: row.get(10)?,
                error_code: row.get(11)?,
                recovered_bytes: row.get(12)?,
            })
        })?;

        let mut files = Vec::new();
        for f in rows {
            files.push(f?);
        }

        let report = serde_json::json!({
            "job": job,
            "files": files,
            "errors": errors,
        });

        serde_json::to_string_pretty(&report).map_err(|e| JournalError::CorruptState(e.to_string()))
    }

    /// Exports job report in CSV format.
    pub fn export_job_report_csv(&self, job_id: &str) -> Result<String, JournalError> {
        let _job = self
            .get_job(job_id)?
            .ok_or_else(|| JournalError::JobNotFound(job_id.to_string()))?;

        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"SELECT id, relative_path, source_size, state, verified_digest, recovered_bytes, error_code
               FROM files WHERE job_id = ?1 ORDER BY id ASC"#,
        )?;

        let mut csv =
            String::from("id,relative_path,source_size,state,digest,recovered_bytes,error_code\n");
        let rows = stmt.query_map(params![job_id], |row| {
            let id: i64 = row.get(0)?;
            let path: String = row.get(1)?;
            let size: u64 = row.get(2)?;
            let state: String = row.get(3)?;
            let digest: Option<String> = row.get(4)?;
            let recovered: u64 = row.get(5)?;
            let error_code: Option<i32> = row.get(6)?;
            Ok((id, path, size, state, digest, recovered, error_code))
        })?;

        for r in rows {
            let (id, path, size, state, digest, recovered, error_code) = r?;
            csv.push_str(&format!(
                "{},\"{}\",{},{},{},{},{}\n",
                id,
                path.replace('"', "\"\""),
                size,
                state,
                digest.as_deref().unwrap_or(""),
                recovered,
                error_code.map(|c| c.to_string()).unwrap_or_default(),
            ));
        }

        Ok(csv)
    }
}
