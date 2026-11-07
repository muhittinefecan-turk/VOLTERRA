use crate::db::JournalEngine;
use crate::error::JournalError;
use crate::models::{
    ChunkRecord, FileRecord, FileResumptionPlan, FileState, JobState, ReconciliationReport,
};
use rusqlite::{params, OptionalExtension};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

impl JournalEngine {
    /// Reconciles job state after a crash, abnormal termination, or unmount.
    /// Checks actual files on disk against SQLite journal records and rolls back
    /// incomplete or corrupt chunks while preserving verified ones.
    pub fn reconcile_job_after_crash(
        &self,
        job_id: &str,
        destination_root: &Path,
        verify_digests: bool,
    ) -> Result<ReconciliationReport, JournalError> {
        let job = self
            .get_job(job_id)?
            .ok_or_else(|| JournalError::JobNotFound(job_id.to_string()))?;

        // 1. Transition job to Paused if it was Running or Scanning
        if job.state == JobState::Running || job.state == JobState::Scanning {
            self.update_job_status(job_id, JobState::Paused)?;
        }

        // 2. Query interrupted files
        let interrupted_files: Vec<FileRecord> = {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare(
                r#"SELECT id, job_id, relative_path, source_size, source_mtime_ns, source_btime_ns,
                          posix_mode, source_file_id, temp_path, state, verified_digest,
                          error_code, recovered_bytes
                   FROM files
                   WHERE job_id = ?1 AND state IN ('Copying', 'Verifying', 'Partial')"#,
            )?;

            let rows = stmt.query_map(params![job_id], |row| {
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
                    state: FileState::Copying,
                    verified_digest: row.get(10)?,
                    error_code: row.get(11)?,
                    recovered_bytes: row.get(12)?,
                })
            })?;

            let mut list = Vec::new();
            for r in rows {
                list.push(r?);
            }
            list
        };

        let mut reconciled_files_count = 0;
        let mut reverted_chunks_count = 0;
        let mut verified_chunks_count = 0;

        for file in &interrupted_files {
            reconciled_files_count += 1;

            let partial_path = match &file.temp_path {
                Some(p) => destination_root.join(p),
                None => destination_root.join(format!(
                    ".volterra.{}.partial",
                    file.relative_path.display()
                )),
            };

            let verified_chunks = self.get_verified_chunks(file.id)?;

            if !partial_path.exists() {
                // Disk file is completely missing; reset all chunks and file to Pending
                for chunk in verified_chunks {
                    self.reset_chunk_to_pending(chunk.file_id, chunk.chunk_index)?;
                    reverted_chunks_count += 1;
                }
                self.update_file_status(file.id, FileState::Pending)?;
                self.update_file_recovered_bytes(file.id, 0)?;
            } else {
                let disk_len = std::fs::metadata(&partial_path)
                    .map(|m| m.len())
                    .unwrap_or(0);

                let mut surviving_verified_bytes: u64 = 0;

                for chunk in verified_chunks {
                    let chunk_end = chunk.offset_bytes + (chunk.length_bytes as u64);

                    if chunk_end > disk_len {
                        // File was truncated before chunk was fully flushed to disk
                        self.reset_chunk_to_pending(chunk.file_id, chunk.chunk_index)?;
                        reverted_chunks_count += 1;
                    } else if verify_digests && chunk.digest.is_some() {
                        let expected_digest = chunk.digest.as_ref().unwrap();
                        let matches = verify_chunk_on_disk(&partial_path, &chunk, expected_digest);
                        if matches {
                            surviving_verified_bytes += chunk.length_bytes as u64;
                            verified_chunks_count += 1;
                        } else {
                            self.reset_chunk_to_pending(chunk.file_id, chunk.chunk_index)?;
                            reverted_chunks_count += 1;
                        }
                    } else {
                        surviving_verified_bytes += chunk.length_bytes as u64;
                        verified_chunks_count += 1;
                    }
                }

                self.update_file_recovered_bytes(file.id, surviving_verified_bytes)?;
                if surviving_verified_bytes > 0 {
                    self.update_file_status(file.id, FileState::Partial)?;
                } else {
                    self.update_file_status(file.id, FileState::Pending)?;
                }
            }
        }

        // Recalculate total copied bytes for the job
        let total_copied = self.recalculate_job_copied_bytes(job_id)?;

        Ok(ReconciliationReport {
            job_id: job_id.to_string(),
            reconciled_files_count,
            reverted_chunks_count,
            verified_chunks_count,
            recovered_bytes: total_copied,
        })
    }

    /// Computes the missing byte extents for an interrupted or partial file,
    /// enabling the reader pipeline to reuse already verified chunks.
    pub fn get_resumption_plan(&self, file_id: i64) -> Result<FileResumptionPlan, JournalError> {
        let (relative_path, source_size) = {
            let conn = self.conn.lock();
            let mut stmt =
                conn.prepare("SELECT relative_path, source_size FROM files WHERE id = ?1")?;
            stmt.query_row(params![file_id], |row| {
                let path_str: String = row.get(0)?;
                let size: u64 = row.get(1)?;
                Ok((PathBuf::from(path_str), size))
            })
            .optional()?
            .ok_or(JournalError::FileNotFound(file_id))?
        };

        let verified_chunks = self.get_verified_chunks(file_id)?;

        let mut verified_extents: Vec<(u64, u64)> = verified_chunks
            .into_iter()
            .map(|c| (c.offset_bytes, c.length_bytes as u64))
            .collect();

        verified_extents.sort_by_key(|&(offset, _)| offset);

        let mut missing_extents = Vec::new();
        let mut cursor = 0u64;

        for &(offset, len) in &verified_extents {
            if offset > cursor {
                missing_extents.push((cursor, offset - cursor));
            }
            cursor = cursor.max(offset + len);
        }

        if cursor < source_size {
            missing_extents.push((cursor, source_size - cursor));
        }

        let already_verified_bytes: u64 = verified_extents.iter().map(|&(_, len)| len).sum();
        let remaining_bytes = source_size.saturating_sub(already_verified_bytes);

        Ok(FileResumptionPlan {
            file_id,
            relative_path,
            total_size: source_size,
            verified_extents,
            missing_extents,
            already_verified_bytes,
            remaining_bytes,
        })
    }

    fn reset_chunk_to_pending(&self, file_id: i64, chunk_index: u32) -> Result<(), JournalError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE chunks SET state = 'Pending' WHERE file_id = ?1 AND chunk_index = ?2",
            params![file_id, chunk_index],
        )?;
        Ok(())
    }

    fn update_file_recovered_bytes(
        &self,
        file_id: i64,
        recovered_bytes: u64,
    ) -> Result<(), JournalError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE files SET recovered_bytes = ?1 WHERE id = ?2",
            params![recovered_bytes, file_id],
        )?;
        Ok(())
    }

    fn recalculate_job_copied_bytes(&self, job_id: &str) -> Result<u64, JournalError> {
        let conn = self.conn.lock();
        let total: u64 = conn.query_row(
            r#"SELECT COALESCE(SUM(c.length_bytes), 0)
               FROM chunks c
               JOIN files f ON c.file_id = f.id
               WHERE f.job_id = ?1 AND c.state = 'Verified'"#,
            params![job_id],
            |row| row.get(0),
        )?;

        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE jobs SET copied_bytes = ?1, updated_at = ?2 WHERE id = ?3",
            params![total, now, job_id],
        )?;

        Ok(total)
    }
}

fn verify_chunk_on_disk(partial_path: &Path, chunk: &ChunkRecord, expected_digest: &str) -> bool {
    let mut file = match File::open(partial_path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    if file.seek(SeekFrom::Start(chunk.offset_bytes)).is_err() {
        return false;
    }

    let mut buf = vec![0u8; chunk.length_bytes as usize];
    if file.read_exact(&mut buf).is_err() {
        return false;
    }

    let actual_digest = volterra_integrity::blake3::Blake3Hasher::hash_chunk(&buf);
    let actual_hex = volterra_integrity::blake3::HashDigest(actual_digest).to_hex();

    actual_hex.eq_ignore_ascii_case(expected_digest)
}
