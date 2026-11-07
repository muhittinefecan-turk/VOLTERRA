use crate::atomic_publish::AtomicPublisher;
use crate::buffer::ChunkBufferPool;
use crate::error::TransferError;
use crate::reader::{ChunkReader, ReadFaultInjector};
use crate::writer::{ChunkWriter, WriteFaultInjector};
use std::collections::HashSet;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use volterra_integrity::{Blake3FileHasher, HashDigest};
use volterra_journal::{ChunkRecord, ChunkState, ExtentErrorRecord, FileState, JournalStorage};
use volterra_manifest::{ItemType, Manifest, ManifestCatalog, ManifestItem};
use volterra_platform_macos::{PlatformAtomic, PlatformFs, PlatformSymlink};
use volterra_profiler::PerformanceMonitor;
use volterra_recovery::{
    ErrorDomain, ExtentInterval, ExtentMap, QuarantineState, QuarantinedFile, RecoveryClassifier,
    RecoveryController,
};

#[derive(Debug, Clone)]
pub struct TransferSchedulerConfig {
    pub source_root: PathBuf,
    pub target_root: PathBuf,
    pub job_id: String,
    pub chunk_size: usize,
    pub memory_budget_bytes: usize,
    pub direct_io: bool,
    pub verify_readback: bool,
    pub allow_overwrite: bool,
}

pub struct TransferScheduler {
    config: TransferSchedulerConfig,
    journal: Arc<dyn JournalStorage>,
    recovery: Arc<RecoveryController>,
    monitor: Arc<PerformanceMonitor>,
    paused: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    read_fault_injector: Option<Arc<dyn ReadFaultInjector>>,
    write_fault_injector: Option<Arc<dyn WriteFaultInjector>>,
}

impl TransferScheduler {
    pub fn new(
        config: TransferSchedulerConfig,
        journal: Arc<dyn JournalStorage>,
        recovery: Arc<RecoveryController>,
        monitor: Arc<PerformanceMonitor>,
        paused: Arc<AtomicBool>,
        cancelled: Arc<AtomicBool>,
    ) -> Self {
        Self {
            config,
            journal,
            recovery,
            monitor,
            paused,
            cancelled,
            read_fault_injector: None,
            write_fault_injector: None,
        }
    }

    pub fn with_read_fault_injector(mut self, injector: Arc<dyn ReadFaultInjector>) -> Self {
        self.read_fault_injector = Some(injector);
        self
    }

    pub fn with_write_fault_injector(mut self, injector: Arc<dyn WriteFaultInjector>) -> Self {
        self.write_fault_injector = Some(injector);
        self
    }

    /// Preflight capacity and permission verification.
    pub fn preflight_check(&self, manifest: &Manifest) -> Result<(), TransferError> {
        let stats = PlatformFs::query_stats(&self.config.target_root)?;
        if stats.is_readonly {
            return Err(TransferError::TargetReadOnly(
                self.config.target_root.clone(),
            ));
        }
        if stats.available_bytes < manifest.total_bytes {
            return Err(TransferError::PreflightCapacityExceeded {
                required_bytes: manifest.total_bytes,
                available_bytes: stats.available_bytes,
                target_path: self.config.target_root.clone(),
            });
        }
        Ok(())
    }

    /// Executes the complete scheduled transfer pipeline over the manifest.
    pub fn execute_pipeline<FProgress>(
        &self,
        manifest: &Manifest,
        mut on_progress: FProgress,
    ) -> Result<(), TransferError>
    where
        FProgress: FnMut(i64, u32, u64, u64),
    {
        self.preflight_check(manifest)?;

        // Pre-create directory tree structure
        for item in &manifest.items {
            let target_path = self.config.target_root.join(&item.relative_path);
            match item.item_type {
                ItemType::Directory => {
                    std::fs::create_dir_all(&target_path).map_err(|e| {
                        TransferError::DestinationWriteError {
                            offset: 0,
                            len: 0,
                            source: e,
                        }
                    })?;
                }
                ItemType::Symlink => {
                    if let Some(target) = &item.symlink_target {
                        let _ = PlatformSymlink::create_symlink(target, &target_path);
                    }
                }
                _ => {}
            }
        }

        let order = ManifestCatalog::order_for_hdd(manifest);
        let buffer_pool = Arc::new(ChunkBufferPool::new(
            self.config.chunk_size,
            self.config.memory_budget_bytes,
        ));

        for &idx in &order {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(TransferError::Cancelled);
            }

            let item = &manifest.items[idx];
            if item.item_type != ItemType::RegularFile {
                continue;
            }

            self.transfer_single_file(item, &buffer_pool, &mut on_progress)?;
        }

        Ok(())
    }

    fn transfer_single_file<FProgress>(
        &self,
        item: &ManifestItem,
        buffer_pool: &Arc<ChunkBufferPool>,
        on_progress: &mut FProgress,
    ) -> Result<(), TransferError>
    where
        FProgress: FnMut(i64, u32, u64, u64),
    {
        let src_path = self.config.source_root.join(&item.relative_path);
        let dst_path = self.config.target_root.join(&item.relative_path);
        let partial_path = PlatformAtomic::make_partial_path(&dst_path, &self.config.job_id);

        if dst_path.exists() && !self.config.allow_overwrite {
            return Err(TransferError::DestinationCollision(dst_path));
        }

        let mut reader = match ChunkReader::open(&src_path, self.config.direct_io) {
            Ok(r) => r,
            Err(e) => {
                self.monitor.record_read_error();
                let err_msg = e.to_string();
                let errno = e.raw_os_error();
                let is_hardware = errno == Some(libc::EIO);

                let error_record = ExtentErrorRecord {
                    id: None,
                    job_id: self.config.job_id.clone(),
                    file_id: Some(item.id),
                    offset_bytes: 0,
                    length_bytes: item.size_bytes,
                    phase: "open".to_string(),
                    errno_code: errno.unwrap_or(0),
                    error_type: if is_hardware {
                        "DeviceIo".to_string()
                    } else {
                        "OpenError".to_string()
                    },
                    attempts: 1,
                    message: err_msg.clone(),
                    action_taken: None,
                    recorded_at: chrono::Utc::now().to_rfc3339(),
                };
                let _ = self.journal.record_extent_error(&error_record);
                let _ = self
                    .journal
                    .update_file_status(item.id, FileState::ReadError);

                let mut failed_extents = ExtentMap::new();
                if item.size_bytes > 0 {
                    if let Ok(ext) = ExtentInterval::new(0, item.size_bytes) {
                        failed_extents.insert(ext);
                    }
                }

                let classified = RecoveryClassifier::classify_errno(
                    errno.unwrap_or(libc::EACCES),
                    ErrorDomain::Source,
                    &err_msg,
                );

                let qf = QuarantinedFile {
                    file_id: item.id,
                    job_id: self.config.job_id.clone(),
                    relative_path: item.relative_path.clone(),
                    total_size_bytes: item.size_bytes,
                    failed_extents,
                    last_error: classified,
                    state: QuarantineState::Quarantined,
                    attempts: 1,
                    quarantined_at: chrono::Utc::now().timestamp() as u64,
                };
                let _ = self.recovery.queue().quarantine_file(qf);

                return Ok(());
            }
        };

        if let Some(ref fi) = self.read_fault_injector {
            reader = reader.with_fault_injector(fi.clone());
        }

        let mut writer = ChunkWriter::open_or_create_partial(&partial_path, self.config.direct_io)
            .map_err(|e| TransferError::DestinationWriteError {
                offset: 0,
                len: 0,
                source: e,
            })?;

        if let Some(ref fi) = self.write_fault_injector {
            writer = writer.with_fault_injector(fi.clone());
        }

        if item.size_bytes > 0 {
            writer
                .set_len(item.size_bytes)
                .map_err(|e| TransferError::DestinationWriteError {
                    offset: 0,
                    len: 0,
                    source: e,
                })?;
        }

        let chunks = ManifestCatalog::generate_chunks_for_file(item, self.config.chunk_size);
        let verified_chunks = self
            .journal
            .get_verified_chunks(item.id)
            .unwrap_or_default();
        let verified_indices: HashSet<u32> =
            verified_chunks.into_iter().map(|c| c.chunk_index).collect();

        let mut file_hasher = Blake3FileHasher::new();
        let mut bytes_copied = 0u64;

        for chunk in &chunks {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(TransferError::Cancelled);
            }

            while self.paused.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(50));
                if self.cancelled.load(Ordering::Relaxed) {
                    return Err(TransferError::Cancelled);
                }
            }

            // Resumption optimization: reuse already verified chunk
            if verified_indices.contains(&chunk.chunk_index) {
                let mut buf = buffer_pool.acquire();
                let mut bytes_read = 0;
                while bytes_read < chunk.length_bytes as usize {
                    let to_read = chunk.length_bytes as usize - bytes_read;
                    let n = unsafe {
                        libc::pread(
                            writer.file().as_raw_fd(),
                            buf[bytes_read..].as_mut_ptr() as *mut libc::c_void,
                            to_read,
                            (chunk.offset_bytes + bytes_read as u64) as libc::off_t,
                        )
                    };
                    if n < 0 {
                        let err = std::io::Error::last_os_error();
                        if err.kind() == std::io::ErrorKind::Interrupted {
                            continue;
                        }
                        break;
                    }
                    if n == 0 {
                        break;
                    }
                    bytes_read += n as usize;
                }
                if bytes_read > 0 {
                    file_hasher.update(&buf[..bytes_read]);
                }
                buffer_pool.release(buf);

                bytes_copied += chunk.length_bytes as u64;
                on_progress(item.id, chunk.chunk_index, bytes_copied, item.size_bytes);
                continue;
            }

            let mut buf = buffer_pool.acquire();
            self.monitor.update_buffer_usage(
                buffer_pool.in_flight_count() * buffer_pool.chunk_size(),
                self.config.memory_budget_bytes,
            );

            let t_read = Instant::now();
            let read_result =
                reader.read_chunk(chunk.offset_bytes, chunk.length_bytes as usize, &mut buf);
            let read_latency = t_read.elapsed().as_nanos() as u64;

            match read_result {
                Ok(digest) => {
                    self.monitor
                        .record_read(chunk.length_bytes as usize, read_latency);

                    let chunk_slice = &buf[..chunk.length_bytes as usize];
                    let is_all_zeros =
                        !chunk_slice.is_empty() && chunk_slice.iter().all(|&b| b == 0);

                    if is_all_zeros {
                        // Skip physical pwrite; writer.set_len preserves unallocated APFS holes
                        file_hasher.update(chunk_slice);
                        buffer_pool.release(buf);

                        let record = ChunkRecord {
                            file_id: item.id,
                            chunk_index: chunk.chunk_index,
                            offset_bytes: chunk.offset_bytes,
                            length_bytes: chunk.length_bytes,
                            digest: Some(HashDigest::new(digest).to_hex()),
                            state: ChunkState::Verified,
                            attempts: 1,
                        };
                        let _ = self.journal.record_chunk_verified(&record);

                        bytes_copied += chunk.length_bytes as u64;
                        on_progress(item.id, chunk.chunk_index, bytes_copied, item.size_bytes);
                        continue;
                    }

                    let t_write = Instant::now();
                    let write_res =
                        writer.write_chunk(chunk.offset_bytes, chunk_slice);
                    let write_latency = t_write.elapsed().as_nanos() as u64;

                    if let Err(e) = write_res {
                        buffer_pool.release(buf);
                        self.monitor.record_write_error();
                        return Err(e);
                    }

                    self.monitor
                        .record_write(chunk.length_bytes as usize, write_latency);
                    file_hasher.update(chunk_slice);
                    buffer_pool.release(buf);

                    let record = ChunkRecord {
                        file_id: item.id,
                        chunk_index: chunk.chunk_index,
                        offset_bytes: chunk.offset_bytes,
                        length_bytes: chunk.length_bytes,
                        digest: Some(HashDigest::new(digest).to_hex()),
                        state: ChunkState::Verified,
                        attempts: 1,
                    };
                    let _ = self.journal.record_chunk_verified(&record);

                    bytes_copied += chunk.length_bytes as u64;
                    on_progress(item.id, chunk.chunk_index, bytes_copied, item.size_bytes);
                }
                Err(err) => {
                    buffer_pool.release(buf);
                    self.monitor.record_read_error();

                    let err_msg = err.to_string();
                    let (errno, is_hardware) = match &err {
                        TransferError::SourceReadError { source, .. } => (
                            source.raw_os_error(),
                            source.raw_os_error() == Some(libc::EIO),
                        ),
                        _ => (None, false),
                    };

                    let error_record = ExtentErrorRecord {
                        id: None,
                        job_id: self.config.job_id.clone(),
                        file_id: Some(item.id),
                        offset_bytes: chunk.offset_bytes,
                        length_bytes: chunk.length_bytes as u64,
                        phase: "read".to_string(),
                        errno_code: errno.unwrap_or(0),
                        error_type: if is_hardware {
                            "DeviceIo".to_string()
                        } else {
                            "ReadError".to_string()
                        },
                        attempts: 1,
                        message: err_msg.clone(),
                        action_taken: None,
                        recorded_at: chrono::Utc::now().to_rfc3339(),
                    };
                    let _ = self.journal.record_extent_error(&error_record);
                    let _ = self
                        .journal
                        .update_file_status(item.id, FileState::ReadError);

                    // Isolate into recovery quarantine
                    let mut failed_extents = ExtentMap::new();
                    if let Ok(ext) =
                        ExtentInterval::new(chunk.offset_bytes, chunk.length_bytes as u64)
                    {
                        failed_extents.insert(ext);
                    }

                    let classified = RecoveryClassifier::classify_errno(
                        errno.unwrap_or(libc::EIO),
                        ErrorDomain::Source,
                        &err_msg,
                    );

                    let qf = QuarantinedFile {
                        file_id: item.id,
                        job_id: self.config.job_id.clone(),
                        relative_path: item.relative_path.clone(),
                        total_size_bytes: item.size_bytes,
                        failed_extents,
                        last_error: classified,
                        state: QuarantineState::Quarantined,
                        attempts: 1,
                        quarantined_at: chrono::Utc::now().timestamp() as u64,
                    };
                    let _ = self.recovery.queue().quarantine_file(qf);

                    // Error isolation: Healthy files continue without aborting the entire job
                    return Ok(());
                }
            }
        }

        let final_digest = file_hasher.finalize_digest();
        AtomicPublisher::publish_file(
            writer.file(),
            writer.path(),
            &dst_path,
            &self.config.source_root,
            item,
            item.size_bytes,
            Some(final_digest.as_bytes()),
            self.config.verify_readback,
            self.config.allow_overwrite,
        )?;

        let _ =
            self.journal
                .update_file_completion(item.id, &final_digest.to_hex(), item.size_bytes);

        Ok(())
    }
}
