use crate::error::PlatformError;
use std::path::{Path, PathBuf};

pub struct PlatformSymlink;

impl PlatformSymlink {
    /// Determines whether the path is a symbolic link without following it.
    pub fn is_symlink(path: &Path) -> bool {
        std::fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }

    /// Reads the raw target path pointed to by the symlink.
    pub fn read_symlink(path: &Path) -> Result<PathBuf, PlatformError> {
        std::fs::read_link(path).map_err(|e| PlatformError::SymlinkError {
            path: path.display().to_string(),
            reason: e.to_string(),
        })
    }

    /// Creates a new symlink at `link_path` pointing to `target`.
    pub fn create_symlink(target: &Path, link_path: &Path) -> Result<(), PlatformError> {
        if link_path.exists() || Self::is_symlink(link_path) {
            return Err(PlatformError::TargetExists(link_path.to_path_buf()));
        }
        std::os::unix::fs::symlink(target, link_path).map_err(|e| PlatformError::SymlinkError {
            path: link_path.display().to_string(),
            reason: e.to_string(),
        })
    }

    /// Safely preserves a symlink from source to destination without circular traversal.
    pub fn preserve_symlink(src: &Path, dst: &Path) -> Result<(), PlatformError> {
        let target = Self::read_symlink(src)?;
        Self::create_symlink(&target, dst)
    }
}
