use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use volterra_journal::JournalEngine;
use volterra_manifest::{Manifest, ManifestScanner, ScanOptions};

/// Hermetic test environment containing isolated temporary directories
/// for source, target destination, and SQLite journal.
pub struct TestEnvironment {
    pub src_dir: TempDir,
    pub dst_dir: TempDir,
    pub journal_dir: TempDir,
}

impl TestEnvironment {
    pub fn new() -> Self {
        Self {
            src_dir: TempDir::new().expect("Failed to create temporary source directory"),
            dst_dir: TempDir::new().expect("Failed to create temporary destination directory"),
            journal_dir: TempDir::new().expect("Failed to create temporary journal directory"),
        }
    }

    pub fn src_path(&self) -> &Path {
        self.src_dir.path()
    }

    pub fn dst_path(&self) -> &Path {
        self.dst_dir.path()
    }

    pub fn journal_db_path(&self) -> PathBuf {
        self.journal_dir.path().join("volterra_test.db")
    }

    pub fn open_journal(&self) -> Arc<JournalEngine> {
        let db_path = self.journal_db_path();
        Arc::new(JournalEngine::open(&db_path).expect("Failed to initialize test journal"))
    }

    /// Creates a deterministic file with specific size and byte pattern
    pub fn create_file(&self, rel_path: &str, size_bytes: usize, pattern_byte: u8) -> PathBuf {
        let full_path = self.src_path().join(rel_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create parent directories");
        }
        let data = vec![pattern_byte; size_bytes];
        let mut f = File::create(&full_path).expect("Failed to create test file");
        f.write_all(&data).expect("Failed to write test data");
        f.sync_all().expect("Failed to sync file to disk");
        full_path
    }

    /// Creates a deterministic file with pseudorandom pattern based on index
    pub fn create_pattern_file(&self, rel_path: &str, size_bytes: usize) -> PathBuf {
        let full_path = self.src_path().join(rel_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create parent directories");
        }
        let mut data = Vec::with_capacity(size_bytes);
        for i in 0..size_bytes {
            data.push(((i ^ (i >> 8)) & 0xFF) as u8);
        }
        let mut f = File::create(&full_path).expect("Failed to create test file");
        f.write_all(&data).expect("Failed to write test data");
        f.sync_all().expect("Failed to sync file to disk");
        full_path
    }

    /// Scans the source directory and returns the manifest
    pub fn scan_source(&self) -> Manifest {
        ManifestScanner::scan_tree(self.src_path(), &ScanOptions::default())
            .expect("Failed to scan test manifest")
    }
}
