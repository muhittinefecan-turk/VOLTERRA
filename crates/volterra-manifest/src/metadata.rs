use crate::error::ManifestError;
use crate::model::ManifestItem;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::RawFd;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationStatus {
    Unmodified,
    IdentityChanged {
        expected_ino: u64,
        actual_ino: u64,
    },
    SizeChanged {
        original: u64,
        current: u64,
    },
    MtimeChanged {
        original_sec: i64,
        current_sec: i64,
        original_nsec: i64,
        current_nsec: i64,
    },
    Deleted,
}

impl MutationStatus {
    pub fn is_unmodified(&self) -> bool {
        matches!(self, MutationStatus::Unmodified)
    }

    pub fn is_mutated(&self) -> bool {
        !self.is_unmodified()
    }
}

#[derive(Debug, Clone)]
pub struct SourceAccessReport {
    pub root_accessible: bool,
    pub is_read_only_fs: bool,
    pub total_files_checked: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBaseline {
    pub file_id: String,
    pub dev: u64,
    pub inode: u64,
    pub size: u64,
    pub mtime_ns: i64,
    pub mtime_sec: i64,
}

impl SourceBaseline {
    pub fn from_item(item: &ManifestItem) -> Self {
        Self {
            file_id: item.source_file_id.clone(),
            dev: item.dev,
            inode: item.inode,
            size: item.size_bytes,
            mtime_ns: item.mtime_nsec,
            mtime_sec: item.mtime_sec,
        }
    }

    pub fn expected_dev_ino(&self) -> (u64, u64) {
        if self.dev != 0 || self.inode != 0 {
            (self.dev, self.inode)
        } else if let Some((d, i)) = self.file_id.split_once(':') {
            (d.parse::<u64>().unwrap_or(0), i.parse::<u64>().unwrap_or(0))
        } else {
            (0, 0)
        }
    }
}

pub struct SourceValidator;

impl SourceValidator {
    /// Validates that source path is accessible and readable with O_RDONLY.
    pub fn validate_source_access(root: &Path) -> Result<SourceAccessReport, ManifestError> {
        if !root.exists() {
            return Err(ManifestError::RootNotFound(root.to_path_buf()));
        }

        let rd = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(ManifestError::PermissionDenied(root.to_path_buf()));
            }
            Err(e) => {
                return Err(ManifestError::Io {
                    path: root.to_path_buf(),
                    source: e,
                });
            }
        };

        let mut checked = 0;
        for entry in rd.take(10) {
            if entry.is_ok() {
                checked += 1;
            }
        }

        // Query read-only flag via statfs
        let stats = volterra_platform_macos::PlatformFs::query_stats(root).map_err(|e| {
            ManifestError::Io {
                path: root.to_path_buf(),
                source: std::io::Error::other(e.to_string()),
            }
        })?;

        Ok(SourceAccessReport {
            root_accessible: true,
            is_read_only_fs: stats.is_readonly,
            total_files_checked: checked,
        })
    }

    /// Re-validates that source file did not mutate during transfer prior to publication.
    pub fn revalidate_source_mutation(
        item: &ManifestItem,
        current_full_path: &Path,
    ) -> Result<MutationStatus, ManifestError> {
        let meta = match fs::symlink_metadata(current_full_path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(MutationStatus::Deleted);
            }
            Err(e) => {
                return Err(ManifestError::Io {
                    path: current_full_path.to_path_buf(),
                    source: e,
                });
            }
        };

        // Source file identity check (inode & dev) to prevent silent file replacement
        if meta.ino() != item.inode || meta.dev() != item.dev {
            return Ok(MutationStatus::IdentityChanged {
                expected_ino: item.inode,
                actual_ino: meta.ino(),
            });
        }

        // Size check
        if meta.len() != item.size_bytes {
            return Ok(MutationStatus::SizeChanged {
                original: item.size_bytes,
                current: meta.len(),
            });
        }

        // Nanosecond-precision modification time check
        if meta.mtime() != item.mtime_sec || meta.mtime_nsec() != item.mtime_nsec {
            return Ok(MutationStatus::MtimeChanged {
                original_sec: item.mtime_sec,
                current_sec: meta.mtime(),
                original_nsec: item.mtime_nsec,
                current_nsec: meta.mtime_nsec(),
            });
        }

        Ok(MutationStatus::Unmodified)
    }

    /// Validates that source path matches baseline, returning Err(SourceMutated) if changed.
    pub fn check_unmodified(
        source_path: &Path,
        baseline: &SourceBaseline,
    ) -> Result<(), ManifestError> {
        let meta = match fs::symlink_metadata(source_path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ManifestError::SourceDeleted {
                    path: source_path.to_path_buf(),
                });
            }
            Err(e) => {
                return Err(ManifestError::Io {
                    path: source_path.to_path_buf(),
                    source: e,
                });
            }
        };

        // Inode & dev identity validation
        let (expected_dev, expected_ino) = baseline.expected_dev_ino();
        if (expected_dev != 0 || expected_ino != 0)
            && (meta.dev() != expected_dev || meta.ino() != expected_ino)
        {
            return Err(ManifestError::SourceMutated {
                path: source_path.to_path_buf(),
                reason: format!(
                    "File identity changed: expected dev:ino {}:{}, found {}:{}",
                    expected_dev,
                    expected_ino,
                    meta.dev(),
                    meta.ino()
                ),
            });
        }

        // Size check
        if meta.len() != baseline.size {
            return Err(ManifestError::SourceMutated {
                path: source_path.to_path_buf(),
                reason: format!(
                    "File size changed: expected {} bytes, found {} bytes",
                    baseline.size,
                    meta.len()
                ),
            });
        }

        // Modification time check (seconds and subsecond nanoseconds)
        if meta.mtime() != baseline.mtime_sec || meta.mtime_nsec() != baseline.mtime_ns {
            return Err(ManifestError::SourceMutated {
                path: source_path.to_path_buf(),
                reason: format!(
                    "Modification time changed: expected {}.{:09} s, found {}.{:09} s",
                    baseline.mtime_sec,
                    baseline.mtime_ns,
                    meta.mtime(),
                    meta.mtime_nsec()
                ),
            });
        }

        Ok(())
    }

    /// Validates an open source file descriptor before publication.
    pub fn check_fd_unmodified(
        fd: RawFd,
        source_path: &Path,
        baseline: &SourceBaseline,
    ) -> Result<(), ManifestError> {
        let mut stat_buf: libc::stat = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::fstat(fd, &mut stat_buf) };

        if ret < 0 {
            return Err(ManifestError::Io {
                path: source_path.to_path_buf(),
                source: std::io::Error::last_os_error(),
            });
        }

        // Inode & dev identity validation
        let (expected_dev, expected_ino) = baseline.expected_dev_ino();
        let current_ino = stat_buf.st_ino as u64;
        let current_dev = stat_buf.st_dev as u64;

        if (expected_dev != 0 || expected_ino != 0)
            && (current_dev != expected_dev || current_ino != expected_ino)
        {
            return Err(ManifestError::SourceMutated {
                path: source_path.to_path_buf(),
                reason: format!(
                    "File descriptor identity mismatch: expected dev:ino {}:{}, found {}:{}",
                    expected_dev, expected_ino, current_dev, current_ino
                ),
            });
        }

        let current_size = stat_buf.st_size as u64;
        if current_size != baseline.size {
            return Err(ManifestError::SourceMutated {
                path: source_path.to_path_buf(),
                reason: format!(
                    "File descriptor size mismatch: expected {} bytes, found {} bytes",
                    baseline.size, current_size
                ),
            });
        }

        let current_mtime_sec = stat_buf.st_mtime as i64;
        let current_mtime_nsec = stat_buf.st_mtime_nsec as i64;
        if current_mtime_sec != baseline.mtime_sec || current_mtime_nsec != baseline.mtime_ns {
            return Err(ManifestError::SourceMutated {
                path: source_path.to_path_buf(),
                reason: format!(
                    "File descriptor mtime mismatch: expected {}.{:09} s, found {}.{:09} s",
                    baseline.mtime_sec, baseline.mtime_ns, current_mtime_sec, current_mtime_nsec
                ),
            });
        }

        Ok(())
    }
}

pub type SourceMutationDetector = SourceValidator;
