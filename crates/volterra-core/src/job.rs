use crate::error::CoreError;
use crate::handle::JobHandle;
use std::path::{Path, PathBuf};
use volterra_journal::JobMode;

pub struct JobBuilder {
    job_id: Option<String>,
    source_root: Option<PathBuf>,
    target_root: Option<PathBuf>,
    mode: JobMode,
    allow_overwrite: bool,
    verify_readback: bool,
    direct_io: bool,
    memory_budget_bytes: usize,
    chunk_size: usize,
    journal_path: Option<PathBuf>,
}

impl Default for JobBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl JobBuilder {
    pub fn new() -> Self {
        Self {
            job_id: None,
            source_root: None,
            target_root: None,
            mode: JobMode::SmartHybrid,
            allow_overwrite: false,
            verify_readback: false,
            direct_io: false,
            memory_budget_bytes: 128 * 1024 * 1024,
            chunk_size: 8 * 1024 * 1024,
            journal_path: None,
        }
    }

    pub fn job_id(mut self, id: impl Into<String>) -> Self {
        self.job_id = Some(id.into());
        self
    }

    pub fn source(mut self, path: impl AsRef<Path>) -> Self {
        self.source_root = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn destination(mut self, path: impl AsRef<Path>) -> Self {
        self.target_root = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn mode(mut self, mode: JobMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn allow_overwrite(mut self, allow: bool) -> Self {
        self.allow_overwrite = allow;
        self
    }

    pub fn verify_readback(mut self, verify: bool) -> Self {
        self.verify_readback = verify;
        self
    }

    pub fn direct_io(mut self, direct: bool) -> Self {
        self.direct_io = direct;
        self
    }

    pub fn memory_budget(mut self, bytes: usize) -> Self {
        self.memory_budget_bytes = std::cmp::max(16 * 1024 * 1024, bytes);
        self
    }

    pub fn chunk_size(mut self, bytes: usize) -> Self {
        self.chunk_size = bytes;
        self
    }

    pub fn journal_path(mut self, path: impl AsRef<Path>) -> Self {
        self.journal_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn build(self) -> Result<JobHandle, CoreError> {
        let source_root = self
            .source_root
            .ok_or(CoreError::MissingParameter("source_root"))?;
        let target_root = self
            .target_root
            .ok_or(CoreError::MissingParameter("target_root"))?;
        let job_id = self
            .job_id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        if !source_root.exists() {
            return Err(CoreError::SourceNotFound(source_root));
        }

        crate::controller::TransferController::create_job(
            job_id,
            source_root,
            target_root,
            self.mode,
            self.allow_overwrite,
            self.verify_readback,
            self.direct_io,
            self.memory_budget_bytes,
            self.chunk_size,
            self.journal_path,
        )
    }
}
