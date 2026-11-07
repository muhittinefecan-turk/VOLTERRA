use crate::error::CoreError;
use crate::events::{EventHub, JobEvent};
use crate::handle::JobHandle;
use crate::snapshot::JobSnapshot;
use crate::state::JobLifecycleState;
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use volterra_journal::{
    FileRecord, FileState, JobMode, JobRecord, JobState, JournalEngine, JournalStorage,
};
use volterra_manifest::{ManifestScanner, ScanOptions};
use volterra_optimizer::{TransferOptimizer, TransferPolicy};
use volterra_profiler::{DiskDiscoveryEngine, PerformanceMonitor};
use volterra_recovery::{ErrorQueue, RecoveryController, RetryBudget};
use volterra_transfer::{TransferScheduler, TransferSchedulerConfig};

pub struct TransferController;

impl TransferController {
    #[allow(clippy::too_many_arguments)]
    pub fn create_job(
        job_id: String,
        source_root: PathBuf,
        target_root: PathBuf,
        mode: JobMode,
        allow_overwrite: bool,
        verify_readback: bool,
        direct_io: bool,
        memory_budget_bytes: usize,
        chunk_size: usize,
        journal_path: Option<PathBuf>,
    ) -> Result<JobHandle, CoreError> {
        let event_hub = Arc::new(EventHub::new());
        let state = Arc::new(RwLock::new(JobLifecycleState::Pending));
        let paused = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        let monitor = Arc::new(PerformanceMonitor::new());

        let db_path = journal_path
            .unwrap_or_else(|| target_root.join(format!(".volterra.{}.journal.db", job_id)));
        let journal: Arc<dyn JournalStorage> = Arc::new(JournalEngine::open(&db_path)?);

        let queue = Arc::new(ErrorQueue::new());
        let recovery = Arc::new(RecoveryController::with_journal(
            queue.clone(),
            RetryBudget::default(),
            journal.clone(),
        ));

        // Evaluate disk relationship
        let relationship = DiskDiscoveryEngine::evaluate_relationship(&source_root, &target_root)
            .unwrap_or_else(|_| volterra_profiler::DiskRelationship {
                source_device_id: "src".into(),
                dest_device_id: "dst".into(),
                source_parent_id: "p0".into(),
                dest_parent_id: "p1".into(),
                is_same_physical_disk: false,
                is_same_filesystem_container: false,
                direction: volterra_profiler::TransferDirection::Unknown,
                recommended_concurrency: 1,
                warning: None,
            });

        if let Some(warn) = &relationship.warning {
            event_hub.broadcast(JobEvent::DiskWarning {
                message: warn.clone(),
            });
        }

        let policy = match mode {
            JobMode::Turbo => TransferPolicy::Performance,
            JobMode::SmartHybrid => TransferPolicy::Automatic,
            JobMode::SafeRecovery => TransferPolicy::SafeRecovery,
        };

        let optimizer = Arc::new(parking_lot::Mutex::new(TransferOptimizer::new(
            relationship,
            memory_budget_bytes,
            policy,
        )));

        // Pre-register job record in journal
        journal.create_job(&JobRecord {
            id: job_id.clone(),
            source_root: source_root.clone(),
            target_root: target_root.clone(),
            source_volume_uuid: None,
            target_volume_uuid: None,
            mode,
            state: JobState::Pending,
            total_files: 0,
            total_bytes: 0,
            copied_bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })?;

        // Shared telemetry progress counters
        let total_files_atomic = Arc::new(AtomicU64::new(0));
        let total_bytes_atomic = Arc::new(AtomicU64::new(0));
        let completed_files_atomic = Arc::new(AtomicU64::new(0));
        let transferred_bytes_atomic = Arc::new(AtomicU64::new(0));

        let snapshot_fn: Arc<dyn Fn() -> JobSnapshot + Send + Sync> = {
            let j_id = job_id.clone();
            let st = state.clone();
            let mon = monitor.clone();
            let q = queue.clone();
            let opt = optimizer.clone();
            let tot_f = total_files_atomic.clone();
            let tot_b = total_bytes_atomic.clone();
            let comp_f = completed_files_atomic.clone();
            let xfer_b = transferred_bytes_atomic.clone();

            Arc::new(move || {
                let tel = mon.get_telemetry();
                let cur_state = *st.read();
                let total_f = tot_f.load(Ordering::Relaxed);
                let total_b = tot_b.load(Ordering::Relaxed);
                let comp_files = comp_f.load(Ordering::Relaxed);
                let xfer_bytes = xfer_b.load(Ordering::Relaxed);
                let rem_bytes = total_b.saturating_sub(xfer_bytes);
                let pct = if total_b > 0 {
                    ((xfer_bytes as f64) / (total_b as f64)) * 100.0
                } else if cur_state == JobLifecycleState::Completed {
                    100.0
                } else {
                    0.0
                };

                let eta_sec = if tel.ewma_5s_mbps > 0.1 && rem_bytes > 0 {
                    Some(((rem_bytes as f64) / (tel.ewma_5s_mbps * 1_048_576.0)) as u64)
                } else {
                    None
                };

                let bottleneck = {
                    let opt_guard = opt.lock();
                    let b = opt_guard.detect_bottleneck(&tel);
                    if b.bottleneck_type != volterra_optimizer::BottleneckType::Nominal {
                        Some(b.user_message)
                    } else {
                        None
                    }
                };

                JobSnapshot {
                    job_id: j_id.clone(),
                    state: cur_state,
                    total_files: total_f,
                    completed_files: comp_files,
                    failed_files: tel.read_errors + tel.write_errors,
                    skipped_files: 0,
                    in_flight_files: if cur_state == JobLifecycleState::Running {
                        1
                    } else {
                        0
                    },
                    total_bytes: total_b,
                    transferred_bytes: xfer_bytes,
                    verified_bytes: xfer_bytes,
                    remaining_bytes: rem_bytes,
                    percent_complete: pct,
                    current_throughput_mbps: tel.instantaneous_mbps,
                    ewma_1s_mbps: tel.ewma_1s_mbps,
                    ewma_5s_mbps: tel.ewma_5s_mbps,
                    ewma_30s_mbps: tel.ewma_30s_mbps,
                    p50_read_latency_ms: tel.p50_read_latency_ms,
                    p95_read_latency_ms: tel.p95_read_latency_ms,
                    p50_write_latency_ms: tel.p50_write_latency_ms,
                    p95_write_latency_ms: tel.p95_write_latency_ms,
                    active_chunk_size_bytes: tel.current_chunk_size,
                    active_concurrency: tel.active_concurrency,
                    buffer_utilization_pct: tel.buffer_utilization_pct,
                    estimated_remaining_seconds: eta_sec,
                    active_bottleneck: bottleneck,
                    quarantined_errors_count: q.list_all().len(),
                }
            })
        };

        let start_fn: Arc<dyn Fn() -> Result<(), CoreError> + Send + Sync> = {
            let j_id = job_id.clone();
            let st = state.clone();
            let eh = event_hub.clone();
            let jnl = journal.clone();
            let rec = recovery.clone();
            let mon = monitor.clone();
            let ps = paused.clone();
            let cn = cancelled.clone();
            let opt = optimizer.clone();
            let s_root = source_root.clone();
            let t_root = target_root.clone();
            let snap_getter = snapshot_fn.clone();
            let tot_f = total_files_atomic.clone();
            let tot_b = total_bytes_atomic.clone();
            let comp_f = completed_files_atomic.clone();
            let xfer_b = transferred_bytes_atomic.clone();

            Arc::new(move || {
                let mut state_guard = st.write();
                if *state_guard != JobLifecycleState::Pending {
                    return Err(CoreError::InvalidStateTransition {
                        current: *state_guard,
                        target: JobLifecycleState::Scanning,
                    });
                }

                *state_guard = JobLifecycleState::Scanning;
                eh.broadcast(JobEvent::StateChanged {
                    old_state: JobLifecycleState::Pending,
                    new_state: JobLifecycleState::Scanning,
                });
                drop(state_guard);

                // Spawn worker thread for scanning and transfer pipeline
                let j_id_cl = j_id.clone();
                let st_cl = st.clone();
                let eh_cl = eh.clone();
                let jnl_cl = jnl.clone();
                let rec_cl = rec.clone();
                let mon_cl = mon.clone();
                let ps_cl = ps.clone();
                let cn_cl = cn.clone();
                let _opt_cl = opt.clone();
                let s_root_cl = s_root.clone();
                let t_root_cl = t_root.clone();
                let snap_cl = snap_getter.clone();
                let tot_f_cl = tot_f.clone();
                let tot_b_cl = tot_b.clone();
                let comp_f_cl = comp_f.clone();
                let xfer_b_cl = xfer_b.clone();

                std::thread::spawn(move || {
                    let start_instant = Instant::now();
                    // 1. Scan directory tree
                    let manifest =
                        match ManifestScanner::scan_tree(&s_root_cl, &ScanOptions::default()) {
                            Ok(m) => m,
                            Err(e) => {
                                let mut st_w = st_cl.write();
                                let old = *st_w;
                                *st_w = JobLifecycleState::Failed;
                                eh_cl.broadcast(JobEvent::StateChanged {
                                    old_state: old,
                                    new_state: JobLifecycleState::Failed,
                                });
                                eh_cl.broadcast(JobEvent::JobFailed {
                                    error: e.to_string(),
                                });
                                return;
                            }
                        };

                    tot_f_cl.store(manifest.file_count, Ordering::Relaxed);
                    tot_b_cl.store(manifest.total_bytes, Ordering::Relaxed);

                    eh_cl.broadcast(JobEvent::ScanningComplete {
                        total_files: manifest.file_count,
                        total_bytes: manifest.total_bytes,
                    });

                    // 2. Persist files batch to SQLite WAL journal
                    let file_records: Vec<_> = manifest
                        .items
                        .iter()
                        .map(|item| FileRecord {
                            id: item.id,
                            job_id: j_id_cl.clone(),
                            relative_path: item.relative_path.clone(),
                            source_size: item.size_bytes,
                            source_mtime_ns: item.source_mtime_ns,
                            source_btime_ns: item.source_btime_ns,
                            posix_mode: item.posix_mode,
                            source_file_id: item.source_file_id.clone(),
                            temp_path: None,
                            state: FileState::Pending,
                            verified_digest: None,
                            error_code: None,
                            recovered_bytes: 0,
                        })
                        .collect();

                    let _ = jnl_cl.record_files_batch(&j_id_cl, &file_records);

                    // 3. Transition to Running
                    {
                        let mut st_w = st_cl.write();
                        let old = *st_w;
                        *st_w = JobLifecycleState::Running;
                        eh_cl.broadcast(JobEvent::StateChanged {
                            old_state: old,
                            new_state: JobLifecycleState::Running,
                        });
                    }

                    // 4. Configure and run transfer scheduler pipeline
                    let sched_config = TransferSchedulerConfig {
                        source_root: s_root_cl,
                        target_root: t_root_cl,
                        job_id: j_id_cl.clone(),
                        chunk_size,
                        memory_budget_bytes,
                        direct_io,
                        verify_readback,
                        allow_overwrite,
                    };

                    let scheduler = TransferScheduler::new(
                        sched_config,
                        jnl_cl.clone(),
                        rec_cl,
                        mon_cl,
                        ps_cl,
                        cn_cl.clone(),
                    );

                    let eh_progress = eh_cl.clone();
                    let snap_progress = snap_cl.clone();
                    let xfer_b_progress = xfer_b_cl.clone();
                    let exec_result = scheduler.execute_pipeline(
                        &manifest,
                        move |file_id, chunk_idx, copied, total| {
                            xfer_b_progress.store(copied, Ordering::Relaxed);
                            eh_progress.broadcast(JobEvent::ChunkProgress {
                                file_id,
                                chunk_index: chunk_idx,
                                bytes_copied: copied,
                                total_bytes: total,
                            });

                            // Emit periodic telemetry snapshot
                            if chunk_idx % 4 == 0 {
                                let snap = snap_progress();
                                eh_progress.broadcast(JobEvent::TelemetryUpdated {
                                    snapshot: Box::new(snap),
                                });
                            }
                        },
                    );

                    // 5. Finalize state
                    let mut st_w = st_cl.write();
                    let old = *st_w;

                    if cn_cl.load(Ordering::Relaxed) {
                        if old != JobLifecycleState::Cancelled {
                            *st_w = JobLifecycleState::Cancelled;
                            eh_cl.broadcast(JobEvent::StateChanged {
                                old_state: old,
                                new_state: JobLifecycleState::Cancelled,
                            });
                        }
                    } else {
                        match exec_result {
                            Ok(()) => {
                                *st_w = JobLifecycleState::Completed;
                                comp_f_cl.store(manifest.file_count, Ordering::Relaxed);
                                xfer_b_cl.store(manifest.total_bytes, Ordering::Relaxed);
                                eh_cl.broadcast(JobEvent::StateChanged {
                                    old_state: old,
                                    new_state: JobLifecycleState::Completed,
                                });
                                eh_cl.broadcast(JobEvent::JobCompleted {
                                    total_files: manifest.file_count,
                                    total_bytes: manifest.total_bytes,
                                    duration_ms: start_instant.elapsed().as_millis() as u64,
                                });
                            }
                            Err(e) => {
                                *st_w = JobLifecycleState::Failed;
                                eh_cl.broadcast(JobEvent::StateChanged {
                                    old_state: old,
                                    new_state: JobLifecycleState::Failed,
                                });
                                eh_cl.broadcast(JobEvent::JobFailed {
                                    error: e.to_string(),
                                });
                            }
                        }
                    }
                });

                Ok(())
            })
        };

        Ok(JobHandle {
            job_id,
            state,
            paused,
            cancelled,
            event_hub,
            recovery,
            monitor,
            snapshot_fn,
            start_fn,
        })
    }
}
