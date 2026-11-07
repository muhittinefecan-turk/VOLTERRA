use crate::error::PlatformError;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FsType {
    Apfs,
    HfsPlus,
    ExFat,
    Fat32,
    Ntfs,
    Unknown(String),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VolumeStats {
    pub block_size: u64,
    pub optimal_io_size: u64,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub available_bytes: u64,
    pub total_inodes: u64,
    pub free_inodes: u64,
    pub is_readonly: bool,
    pub fs_type: FsType,
    pub mount_point: PathBuf,
    pub device_name: String,
}

pub struct PlatformFs;

impl PlatformFs {
    pub fn query_stats(path: &Path) -> Result<VolumeStats, PlatformError> {
        let c_path =
            CString::new(path.as_os_str().as_bytes()).map_err(|_| PlatformError::InvalidPath {
                path: path.to_path_buf(),
            })?;

        let mut sfs: libc::statfs = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::statfs(c_path.as_ptr(), &mut sfs) };
        if ret != 0 {
            return Err(PlatformError::SyscallFailed {
                op: "statfs",
                source: std::io::Error::last_os_error(),
            });
        }

        let block_size = sfs.f_bsize as u64;
        let optimal_io_size = if sfs.f_iosize > 0 {
            sfs.f_iosize as u64
        } else {
            1024 * 1024
        };
        let total_bytes = sfs.f_blocks * block_size;
        let free_bytes = sfs.f_bfree * block_size;
        let available_bytes = sfs.f_bavail * block_size;
        let is_readonly = (sfs.f_flags & (libc::MNT_RDONLY as u32)) != 0;

        let fs_type_name = unsafe {
            std::ffi::CStr::from_ptr(sfs.f_fstypename.as_ptr())
                .to_string_lossy()
                .to_lowercase()
        };
        let fs_type = match fs_type_name.as_str() {
            "apfs" => FsType::Apfs,
            "hfs" => FsType::HfsPlus,
            "exfat" => FsType::ExFat,
            "msdos" | "fat" | "fat32" => FsType::Fat32,
            "ntfs" => FsType::Ntfs,
            other => FsType::Unknown(other.to_string()),
        };

        let mount_point = unsafe {
            let bytes = std::ffi::CStr::from_ptr(sfs.f_mntonname.as_ptr()).to_bytes();
            PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
        };
        let device_name = unsafe {
            std::ffi::CStr::from_ptr(sfs.f_mntfromname.as_ptr())
                .to_string_lossy()
                .to_string()
        };

        Ok(VolumeStats {
            block_size,
            optimal_io_size,
            total_bytes,
            free_bytes,
            available_bytes,
            total_inodes: sfs.f_files,
            free_inodes: sfs.f_ffree as u64,
            is_readonly,
            fs_type,
            mount_point,
            device_name,
        })
    }

    pub fn check_capacity(target_path: &Path, required_bytes: u64) -> Result<(), PlatformError> {
        let stats = Self::query_stats(target_path)?;
        if stats.is_readonly {
            return Err(PlatformError::ReadOnlyFilesystem {
                path: target_path.display().to_string(),
            });
        }
        if stats.available_bytes < required_bytes {
            return Err(PlatformError::SyscallFailed {
                op: "check_capacity",
                source: std::io::Error::from_raw_os_error(libc::ENOSPC),
            });
        }
        Ok(())
    }
}
