use crate::error::ManifestError;
use crate::model::{FileState, ItemType, Manifest, ManifestItem};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub follow_symlinks: bool, // Always false by default to prevent cycles
    pub preserve_xattrs: bool,
    pub max_depth: Option<usize>,
    pub skip_hidden: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            follow_symlinks: false,
            preserve_xattrs: true,
            max_depth: None,
            skip_hidden: false,
        }
    }
}

pub struct ManifestScanner;

impl ManifestScanner {
    /// Scans source directory tree and returns a complete `Manifest`.
    pub fn scan_tree(root: &Path, options: &ScanOptions) -> Result<Manifest, ManifestError> {
        let job_id = format!(
            "job_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let items = Self::scan(&job_id, root, options, |_| {})?;

        let mut total_bytes: u64 = 0;
        let mut file_count: u64 = 0;
        let mut dir_count: u64 = 0;
        let mut symlink_count: u64 = 0;
        let mut special_count: u64 = 0;

        for item in &items {
            match item.item_type {
                ItemType::RegularFile => {
                    total_bytes += item.size_bytes;
                    file_count += 1;
                }
                ItemType::Directory => {
                    dir_count += 1;
                }
                ItemType::Symlink => {
                    symlink_count += 1;
                }
                ItemType::Special => {
                    special_count += 1;
                }
            }
        }

        Ok(Manifest {
            root: root.to_path_buf(),
            job_id,
            items,
            total_bytes,
            file_count,
            dir_count,
            symlink_count,
            special_count,
        })
    }

    /// Recursively scans `source_root`, building an ordered list of `ManifestItem`s.
    /// Invariants:
    /// - Cycles are detected via (dev, ino) tracking and never recursed into.
    /// - Symlinks are captured with their target and never followed recursively.
    /// - Traversal uses a heap-allocated queue, avoiding stack overflow on deep trees.
    pub fn scan<F: FnMut(&ManifestItem)>(
        job_id: &str,
        source_root: &Path,
        options: &ScanOptions,
        mut on_item_found: F,
    ) -> Result<Vec<ManifestItem>, ManifestError> {
        if !source_root.exists() {
            return Err(ManifestError::RootNotFound(source_root.to_path_buf()));
        }

        let canonical_root = fs::canonicalize(source_root).map_err(|e| ManifestError::Io {
            path: source_root.to_path_buf(),
            source: e,
        })?;

        let mut items: Vec<ManifestItem> = Vec::new();
        let mut visited_dirs: HashSet<(u64, u64)> = HashSet::new();
        let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
        let mut current_id: i64 = 1;

        // Stat root directory to mark as visited
        let root_meta = fs::symlink_metadata(&canonical_root).map_err(|e| ManifestError::Io {
            path: canonical_root.clone(),
            source: e,
        })?;

        visited_dirs.insert((root_meta.dev(), root_meta.ino()));
        queue.push_back((canonical_root.clone(), 0));

        while let Some((dir_path, depth)) = queue.pop_front() {
            if let Some(max) = options.max_depth {
                if depth > max {
                    continue;
                }
            }

            let entries = match fs::read_dir(&dir_path) {
                Ok(rd) => rd,
                Err(e) => {
                    // If root itself is unreadable, fail fast
                    if depth == 0 {
                        if e.kind() == std::io::ErrorKind::PermissionDenied {
                            return Err(ManifestError::PermissionDenied(dir_path));
                        } else {
                            return Err(ManifestError::Io {
                                path: dir_path,
                                source: e,
                            });
                        }
                    }

                    // For subdirectories (depth > 0), isolate the error without aborting traversal
                    let rel_path = sanitize_relative_path(
                        dir_path.strip_prefix(&canonical_root).unwrap_or(&dir_path),
                    );
                    let err_code = e.raw_os_error().or(Some(libc::EACCES));

                    if let Some(item) = items.iter_mut().rfind(|i| i.relative_path == rel_path) {
                        item.state = FileState::NeedsAttention;
                        item.error_code = err_code;
                        on_item_found(item);
                    } else {
                        let item = ManifestItem {
                            id: current_id,
                            job_id: job_id.to_string(),
                            relative_path: rel_path,
                            item_type: ItemType::Directory,
                            size_bytes: 0,
                            source_size: 0,
                            mtime_sec: 0,
                            mtime_nsec: 0,
                            source_mtime_ns: 0,
                            btime_nsec: None,
                            source_btime_ns: None,
                            inode: 0,
                            dev: 0,
                            source_file_id: String::new(),
                            posix_mode: 0,
                            has_xattrs: false,
                            symlink_target: None,
                            temp_path: None,
                            state: FileState::NeedsAttention,
                            verified_digest: None,
                            error_code: err_code,
                        };
                        current_id += 1;
                        on_item_found(&item);
                        items.push(item);
                    }
                    continue;
                }
            };

            // Collect and sort directory entries for deterministic scan order
            let mut entry_list: Vec<_> = entries.flatten().collect();
            entry_list.sort_by_key(|e| e.file_name());

            for entry in entry_list {
                let entry_path = entry.path();
                let file_name = entry.file_name();
                let file_name_str = file_name.to_string_lossy();

                if options.skip_hidden && file_name_str.starts_with('.') {
                    continue;
                }

                // CRITICAL: symlink_metadata prevents following symlinks!
                let meta = match fs::symlink_metadata(&entry_path) {
                    Ok(m) => m,
                    Err(e) => {
                        let rel = entry_path
                            .strip_prefix(&canonical_root)
                            .unwrap_or(&entry_path)
                            .to_path_buf();
                        let item = ManifestItem {
                            id: current_id,
                            job_id: job_id.to_string(),
                            relative_path: sanitize_relative_path(&rel),
                            item_type: ItemType::RegularFile,
                            size_bytes: 0,
                            source_size: 0,
                            mtime_sec: 0,
                            mtime_nsec: 0,
                            source_mtime_ns: 0,
                            btime_nsec: None,
                            source_btime_ns: None,
                            inode: 0,
                            dev: 0,
                            source_file_id: String::new(),
                            posix_mode: 0,
                            has_xattrs: false,
                            symlink_target: None,
                            temp_path: None,
                            state: FileState::NeedsAttention,
                            verified_digest: None,
                            error_code: e.raw_os_error(),
                        };
                        current_id += 1;
                        on_item_found(&item);
                        items.push(item);
                        continue;
                    }
                };

                let file_type = meta.file_type();
                let rel_path = sanitize_relative_path(
                    entry_path
                        .strip_prefix(&canonical_root)
                        .unwrap_or(&entry_path),
                );

                let dev = meta.dev();
                let ino = meta.ino();
                let file_id_str = format!("{}:{}", dev, ino);
                let size = meta.len();
                let mtime_sec = meta.mtime();
                let mtime_nsec = meta.mtime_nsec();
                let btime_ns = extract_btime(&meta);

                if file_type.is_symlink() {
                    // Symlink: read target; strictly avoid recursing into it
                    let target = fs::read_link(&entry_path).ok();
                    let item = ManifestItem {
                        id: current_id,
                        job_id: job_id.to_string(),
                        relative_path: rel_path,
                        item_type: ItemType::Symlink,
                        size_bytes: size,
                        source_size: size,
                        mtime_sec,
                        mtime_nsec,
                        source_mtime_ns: mtime_nsec,
                        btime_nsec: btime_ns,
                        source_btime_ns: btime_ns,
                        inode: ino,
                        dev,
                        source_file_id: file_id_str,
                        posix_mode: meta.mode(),
                        has_xattrs: false,
                        symlink_target: target,
                        temp_path: None,
                        state: FileState::Pending,
                        verified_digest: None,
                        error_code: None,
                    };
                    current_id += 1;
                    on_item_found(&item);
                    items.push(item);
                } else if file_type.is_dir() {
                    // Directory: Cycle loop check
                    if visited_dirs.contains(&(dev, ino)) {
                        // Directory cycle loop detected! Ignore recursing.
                        continue;
                    }
                    visited_dirs.insert((dev, ino));

                    let has_xa = if options.preserve_xattrs {
                        has_extended_attributes(&entry_path)
                    } else {
                        false
                    };

                    let item = ManifestItem {
                        id: current_id,
                        job_id: job_id.to_string(),
                        relative_path: rel_path,
                        item_type: ItemType::Directory,
                        size_bytes: 0,
                        source_size: 0,
                        mtime_sec,
                        mtime_nsec,
                        source_mtime_ns: mtime_nsec,
                        btime_nsec: btime_ns,
                        source_btime_ns: btime_ns,
                        inode: ino,
                        dev,
                        source_file_id: file_id_str,
                        posix_mode: meta.mode(),
                        has_xattrs: has_xa,
                        symlink_target: None,
                        temp_path: None,
                        state: FileState::Pending,
                        verified_digest: None,
                        error_code: None,
                    };
                    current_id += 1;
                    on_item_found(&item);
                    items.push(item);

                    queue.push_back((entry_path, depth + 1));
                } else if file_type.is_file() {
                    // Regular file (including 0-byte files)
                    let has_xa = if options.preserve_xattrs {
                        has_extended_attributes(&entry_path)
                    } else {
                        false
                    };

                    let item = ManifestItem {
                        id: current_id,
                        job_id: job_id.to_string(),
                        relative_path: rel_path,
                        item_type: ItemType::RegularFile,
                        size_bytes: size,
                        source_size: size,
                        mtime_sec,
                        mtime_nsec,
                        source_mtime_ns: mtime_nsec,
                        btime_nsec: btime_ns,
                        source_btime_ns: btime_ns,
                        inode: ino,
                        dev,
                        source_file_id: file_id_str,
                        posix_mode: meta.mode(),
                        has_xattrs: has_xa,
                        symlink_target: None,
                        temp_path: None,
                        state: FileState::Pending,
                        verified_digest: None,
                        error_code: None,
                    };
                    current_id += 1;
                    on_item_found(&item);
                    items.push(item);
                } else {
                    // Special file (FIFO, socket, dev node)
                    let item = ManifestItem {
                        id: current_id,
                        job_id: job_id.to_string(),
                        relative_path: rel_path,
                        item_type: ItemType::Special,
                        size_bytes: size,
                        source_size: size,
                        mtime_sec,
                        mtime_nsec,
                        source_mtime_ns: mtime_nsec,
                        btime_nsec: btime_ns,
                        source_btime_ns: btime_ns,
                        inode: ino,
                        dev,
                        source_file_id: file_id_str,
                        posix_mode: meta.mode(),
                        has_xattrs: false,
                        symlink_target: None,
                        temp_path: None,
                        state: FileState::Skipped,
                        verified_digest: None,
                        error_code: None,
                    };
                    current_id += 1;
                    on_item_found(&item);
                    items.push(item);
                }
            }
        }

        Ok(items)
    }
}

fn sanitize_relative_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    let trimmed = s.trim_start_matches('/');
    PathBuf::from(trimmed)
}

fn extract_btime(meta: &fs::Metadata) -> Option<i64> {
    meta.created().ok().and_then(|t| {
        t.duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_nanos() as i64)
    })
}

fn has_extended_attributes(path: &Path) -> bool {
    use std::ffi::CString;
    if let Ok(c_path) = CString::new(path.to_string_lossy().as_bytes()) {
        let size = unsafe {
            libc::listxattr(
                c_path.as_ptr(),
                std::ptr::null_mut(),
                0,
                libc::XATTR_NOFOLLOW,
            )
        };
        size > 0
    } else {
        false
    }
}
