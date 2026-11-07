pub mod atomic;
pub mod error;
pub mod fs;
pub mod io;
pub mod perms;
pub mod profile;
pub mod symlink;
pub mod time;
pub mod volume;
pub mod xattr;

pub use atomic::{AtomicReplaceResult, PlatformAtomic, RENAME_EXCL, RENAME_SWAP};
pub use error::PlatformError;
pub use fs::{FsType, PlatformFs, VolumeStats};
pub use io::{PlatformIo, Radvisory, F_GLOBAL_NOCACHE, F_NOCACHE, F_RDADVISE, F_RDAHEAD};
pub use perms::{FilePermissions, PlatformPerms};
pub use profile::{MediaRotation, PlatformProfile};
pub use symlink::PlatformSymlink;
pub use time::{FileTimestamps, PlatformTime};
pub use volume::{PlatformVolume, VolumeUuid};
pub use xattr::{PlatformXattr, XATTR_NOFOLLOW};

use std::path::Path;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DarwinXattr {
    pub name: String,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DarwinMetadataRecord {
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub times: FileTimestamps,
    pub xattrs: Vec<DarwinXattr>,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct MetadataPreserved {
    pub mode_preserved: bool,
    pub times_preserved: bool,
    pub timestamps_preserved: bool,
    pub permissions_preserved: bool,
    pub xattrs_count: usize,
    pub xattrs_preserved_count: usize,
    pub xattrs_dropped_count: usize,
    pub warnings: Vec<String>,
}

pub struct PlatformMetadata;

impl PlatformMetadata {
    /// Reads full Darwin metadata (mode, timestamps, extended attributes).
    pub fn read_metadata(src: &Path) -> Result<DarwinMetadataRecord, PlatformError> {
        let perms = PlatformPerms::get_permissions(src)?;
        let times = PlatformTime::get_timestamps(src)?;
        let xattr_names = PlatformXattr::list_xattrs(src)?;
        let mut xattrs = Vec::with_capacity(xattr_names.len());
        for name in xattr_names {
            if let Ok(Some(val)) = PlatformXattr::get_xattr(src, &name) {
                xattrs.push(DarwinXattr { name, value: val });
            }
        }
        Ok(DarwinMetadataRecord {
            mode: perms.mode,
            uid: perms.uid,
            gid: perms.gid,
            times,
            xattrs,
        })
    }

    /// Applies metadata record to destination path, gracefully recording warnings if unsupported.
    pub fn apply_metadata(
        dst: &Path,
        meta: &DarwinMetadataRecord,
    ) -> Result<MetadataPreserved, PlatformError> {
        let mut report = MetadataPreserved::default();

        // 1. Timestamps
        if let Err(e) = PlatformTime::set_timestamps(dst, &meta.times) {
            report
                .warnings
                .push(format!("Failed to preserve timestamps: {}", e));
        } else {
            report.times_preserved = true;
            report.timestamps_preserved = true;
        }

        // 2. Permissions (mode & ownership)
        if let Err(e) = PlatformPerms::set_permissions(dst, meta.mode) {
            report.warnings.push(format!("Failed to set mode: {}", e));
        } else {
            report.mode_preserved = true;
            report.permissions_preserved = true;
        }

        // 3. Extended attributes
        for xattr in &meta.xattrs {
            match PlatformXattr::set_xattr(dst, &xattr.name, &xattr.value) {
                Ok(()) => {
                    report.xattrs_preserved_count += 1;
                    report.xattrs_count += 1;
                }
                Err(e) => {
                    report.xattrs_dropped_count += 1;
                    report
                        .warnings
                        .push(format!("Failed to set xattr '{}': {}", xattr.name, e));
                }
            }
        }

        Ok(report)
    }

    /// Copies metadata directly from src to dst.
    pub fn copy_metadata(src: &Path, dst: &Path) -> Result<MetadataPreserved, PlatformError> {
        let meta = Self::read_metadata(src)?;
        Self::apply_metadata(dst, &meta)
    }
}
