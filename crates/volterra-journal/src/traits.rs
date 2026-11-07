use crate::db::JournalEngine;
use crate::error::JournalError;
use crate::models::{
    CheckpointMode, ChunkRecord, ExtentErrorRecord, FileRecord, FileState, JobRecord,
    JobRecoveryState, JobState,
};

pub trait JournalStorage: Send + Sync {
    fn create_job(&self, job: &JobRecord) -> Result<(), JournalError>;
    fn update_job_state(
        &self,
        job_id: &str,
        state: JobState,
        copied_bytes: u64,
    ) -> Result<(), JournalError>;
    fn record_files_batch(&self, job_id: &str, files: &[FileRecord]) -> Result<(), JournalError>;
    fn update_file_status(&self, file_id: i64, status: FileState) -> Result<(), JournalError>;
    fn update_file_completion(
        &self,
        file_id: i64,
        digest: &str,
        recovered_bytes: u64,
    ) -> Result<(), JournalError>;
    fn record_chunk_verified(&self, chunk: &ChunkRecord) -> Result<(), JournalError>;
    fn record_chunks_batch(&self, chunks: &[ChunkRecord]) -> Result<(), JournalError>;
    fn record_extent_error(&self, error: &ExtentErrorRecord) -> Result<i64, JournalError>;
    fn update_extent_error_action(&self, error_id: i64, action: &str) -> Result<(), JournalError>;
    fn load_job_state(&self, job_id: &str) -> Result<JobRecoveryState, JournalError>;
    fn get_verified_chunks(&self, file_id: i64) -> Result<Vec<ChunkRecord>, JournalError>;
    fn get_unverified_chunks(&self, file_id: i64) -> Result<Vec<ChunkRecord>, JournalError>;
    fn get_file_errors(&self, file_id: i64) -> Result<Vec<ExtentErrorRecord>, JournalError>;
    fn get_job_errors(&self, job_id: &str) -> Result<Vec<ExtentErrorRecord>, JournalError>;
    fn checkpoint(&self, mode: CheckpointMode) -> Result<(), JournalError>;
}

impl JournalStorage for JournalEngine {
    fn create_job(&self, job: &JobRecord) -> Result<(), JournalError> {
        self.create_job(job)
    }

    fn update_job_state(
        &self,
        job_id: &str,
        state: JobState,
        copied_bytes: u64,
    ) -> Result<(), JournalError> {
        self.update_job_state(job_id, state, copied_bytes)
    }

    fn record_files_batch(&self, job_id: &str, files: &[FileRecord]) -> Result<(), JournalError> {
        self.record_files_batch(job_id, files)
    }

    fn update_file_status(&self, file_id: i64, status: FileState) -> Result<(), JournalError> {
        self.update_file_status(file_id, status)
    }

    fn update_file_completion(
        &self,
        file_id: i64,
        digest: &str,
        recovered_bytes: u64,
    ) -> Result<(), JournalError> {
        self.update_file_completion(file_id, digest, recovered_bytes)
    }

    fn record_chunk_verified(&self, chunk: &ChunkRecord) -> Result<(), JournalError> {
        self.record_chunk_verified(chunk)
    }

    fn record_chunks_batch(&self, chunks: &[ChunkRecord]) -> Result<(), JournalError> {
        self.record_chunks_batch(chunks)
    }

    fn record_extent_error(&self, error: &ExtentErrorRecord) -> Result<i64, JournalError> {
        self.record_extent_error(error)
    }

    fn update_extent_error_action(&self, error_id: i64, action: &str) -> Result<(), JournalError> {
        self.update_extent_error_action(error_id, action)
    }

    fn load_job_state(&self, job_id: &str) -> Result<JobRecoveryState, JournalError> {
        self.load_job_state(job_id)
    }

    fn get_verified_chunks(&self, file_id: i64) -> Result<Vec<ChunkRecord>, JournalError> {
        self.get_verified_chunks(file_id)
    }

    fn get_unverified_chunks(&self, file_id: i64) -> Result<Vec<ChunkRecord>, JournalError> {
        self.get_unverified_chunks(file_id)
    }

    fn get_file_errors(&self, file_id: i64) -> Result<Vec<ExtentErrorRecord>, JournalError> {
        self.get_file_errors(file_id)
    }

    fn get_job_errors(&self, job_id: &str) -> Result<Vec<ExtentErrorRecord>, JournalError> {
        self.get_job_errors(job_id)
    }

    fn checkpoint(&self, mode: CheckpointMode) -> Result<(), JournalError> {
        self.checkpoint(mode).map(|_| ())
    }
}
