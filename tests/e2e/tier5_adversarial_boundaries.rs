//! Tier 5: Adversarial Boundary, Sparse & Benchmark Challenger Tests
//! Empirically challenges:
//! 1. 0-byte canonical BLAKE3 hash `af1349b...`, 1-byte, 8 MiB chunk boundary, 64 MiB chunk boundary.
//! 2. 25-level deep Turkish Unicode directory tree (`VOLTERRA_Test_Türkçe_ĞÜŞİÖç_Dosyalar`) within Darwin `PATH_MAX = 1024`.
//! 3. 10,000 small files scanning and batch SQLite transaction limits.
//! 4. 1 GiB APFS sparse file with unallocated holes: hole preservation via `set_len`, `st_blocks` sparseness, and 100% BLAKE3 digest match.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use volterra_integrity::Blake3Hasher;
use volterra_journal::{FileRecord, FileState, JobMode, JobRecord, JobState};
use volterra_profiler::PerformanceMonitor;
use volterra_recovery::{RecoveryController, RetryBudget};
use volterra_test_suite::{
    SyntheticDatasetBuilder, SyntheticDatasetValidator, TestEnvironment,
};
use volterra_transfer::{TransferScheduler, TransferSchedulerConfig};

// =========================================================================
// 1. 0-Byte Canonical Hash, 1-Byte, 8 MiB, and 64 MiB Chunk Boundaries
// =========================================================================
#[test]
fn test_challenge_canonical_blake3_empty_and_boundaries() {
    let env = TestEnvironment::new();

    // Verify canonical BLAKE3 empty hash definition
    let empty_digest = Blake3Hasher::hash_to_digest(b"");
    assert_eq!(
        empty_digest.to_hex(),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        "Canonical BLAKE3 hash for empty file must match standard vector"
    );

    // Create 0-byte, 1-byte, exact 8 MiB, and exact 64 MiB boundary files
    let mut builder = SyntheticDatasetBuilder::new(env.src_path());
    builder.add_boundary_files().expect("Create boundary files");

    // Scan source
    let manifest = env.scan_source();
    let journal = env.open_journal();
    let job_id = "job_challenge_boundaries".to_string();

    journal
        .create_job(&JobRecord {
            id: job_id.clone(),
            source_root: env.src_path().to_path_buf(),
            target_root: env.dst_path().to_path_buf(),
            source_volume_uuid: None,
            target_volume_uuid: None,
            mode: JobMode::SmartHybrid,
            state: JobState::Running,
            total_files: manifest.file_count,
            total_bytes: manifest.total_bytes,
            copied_bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })
        .unwrap();

    let file_records: Vec<_> = manifest
        .items
        .iter()
        .map(|item| FileRecord {
            id: item.id,
            job_id: job_id.clone(),
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
    journal.record_files_batch(&job_id, &file_records).unwrap();

    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(volterra_recovery::ErrorQueue::new()),
        RetryBudget::default(),
        journal.clone(),
    ));

    let config = TransferSchedulerConfig {
        source_root: env.src_path().to_path_buf(),
        target_root: env.dst_path().to_path_buf(),
        job_id: job_id.clone(),
        chunk_size: 8 * 1024 * 1024,
        memory_budget_bytes: 32 * 1024 * 1024,
        direct_io: false,
        verify_readback: true,
        allow_overwrite: false,
    };

    let scheduler = TransferScheduler::new(
        config,
        journal,
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );

    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res.is_ok(), "Transfer pipeline failed: {:?}", res);

    // Verify 0-byte destination file
    let empty_dst = env.dst_path().join("boundary_tests").join("empty_0b.bin");
    assert!(empty_dst.exists(), "0-byte destination file must exist");
    assert_eq!(
        fs::metadata(&empty_dst).unwrap().len(),
        0,
        "0-byte destination must have 0 size"
    );
    let mut empty_file = fs::File::open(&empty_dst).unwrap();
    let read_empty_digest = Blake3Hasher::hash_reader(&mut empty_file).unwrap();
    assert_eq!(
        read_empty_digest.to_hex(),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        "0-byte destination file must verify against canonical BLAKE3 hash"
    );

    // Verify all boundary files
    let val = SyntheticDatasetValidator::validate(env.dst_path(), builder.expectations())
        .expect("All boundary files must validate bit-exact");
    assert_eq!(val.files_matched, builder.expectations().len());
}

// =========================================================================
// 2. 25-Level Deep Turkish Unicode Hierarchy within Darwin PATH_MAX = 1024
// =========================================================================
#[test]
fn test_challenge_25_level_turkish_unicode_path_max() {
    let env = TestEnvironment::new();

    let mut builder = SyntheticDatasetBuilder::new(env.src_path());
    builder
        .add_deep_unicode_hierarchy(25)
        .expect("Build 25-level deep Turkish Unicode hierarchy");

    // Verify terminal path length is strictly within Darwin PATH_MAX (1024)
    let terminal_rel = builder
        .expectations()
        .keys()
        .find(|p| p.to_string_lossy().contains("nihai_veri_ğüşıöç.txt"))
        .expect("Terminal file must exist in expectations");
    let terminal_full = env.src_path().join(terminal_rel);
    let path_len = terminal_full.to_string_lossy().len();
    assert!(
        path_len < 1024,
        "Path length ({}) must not exceed Darwin PATH_MAX (1024)",
        path_len
    );

    let manifest = env.scan_source();
    assert!(manifest.file_count >= 2);

    let journal = env.open_journal();
    let job_id = "job_challenge_unicode_25".to_string();

    journal
        .create_job(&JobRecord {
            id: job_id.clone(),
            source_root: env.src_path().to_path_buf(),
            target_root: env.dst_path().to_path_buf(),
            source_volume_uuid: None,
            target_volume_uuid: None,
            mode: JobMode::SmartHybrid,
            state: JobState::Running,
            total_files: manifest.file_count,
            total_bytes: manifest.total_bytes,
            copied_bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })
        .unwrap();

    let file_records: Vec<_> = manifest
        .items
        .iter()
        .map(|item| FileRecord {
            id: item.id,
            job_id: job_id.clone(),
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
    journal.record_files_batch(&job_id, &file_records).unwrap();

    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(volterra_recovery::ErrorQueue::new()),
        RetryBudget::default(),
        journal.clone(),
    ));

    let config = TransferSchedulerConfig {
        source_root: env.src_path().to_path_buf(),
        target_root: env.dst_path().to_path_buf(),
        job_id: job_id.clone(),
        chunk_size: 16 * 1024,
        memory_budget_bytes: 16 * 1024 * 1024,
        direct_io: false,
        verify_readback: true,
        allow_overwrite: false,
    };

    let scheduler = TransferScheduler::new(
        config,
        journal,
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );

    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res.is_ok(), "25-level deep copy failed: {:?}", res);

    let val = SyntheticDatasetValidator::validate(env.dst_path(), builder.expectations())
        .expect("All Turkish Unicode files must validate bit-exact");
    assert_eq!(val.files_matched, builder.expectations().len());
}

// =========================================================================
// 3. 10,000 Small Files Scanning & SQLite Batch Limits (TEST-08)
// =========================================================================
#[test]
fn test_challenge_10k_small_files_scanning_and_sqlite_batch() {
    let env = TestEnvironment::new();

    let total_files = 10_000;
    let dir_fanout = 100;
    let mut builder = SyntheticDatasetBuilder::new(env.src_path());
    builder
        .add_small_files_workload(total_files, dir_fanout)
        .expect("Create 10,000 small files workload");

    assert_eq!(builder.expectations().len(), total_files);

    // 1. Scan source with ManifestScanner
    let manifest = env.scan_source();
    assert_eq!(
        manifest.file_count, total_files as u64,
        "Manifest scanner must detect exactly 10,000 files"
    );

    // 2. Test batch insertion into SQLite WAL journal
    let journal = env.open_journal();
    let job_id = "job_challenge_10k_files".to_string();

    journal
        .create_job(&JobRecord {
            id: job_id.clone(),
            source_root: env.src_path().to_path_buf(),
            target_root: env.dst_path().to_path_buf(),
            source_volume_uuid: None,
            target_volume_uuid: None,
            mode: JobMode::SmartHybrid,
            state: JobState::Running,
            total_files: 0,
            total_bytes: 0,
            copied_bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })
        .unwrap();

    let file_records: Vec<_> = manifest
        .items
        .iter()
        .filter(|item| item.item_type == volterra_manifest::ItemType::RegularFile)
        .map(|item| FileRecord {
            id: item.id,
            job_id: job_id.clone(),
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

    // Verify record_files_batch survives 10,000 records without hitting SQLite parameter limits
    let batch_res = journal.record_files_batch(&job_id, &file_records);
    assert!(
        batch_res.is_ok(),
        "record_files_batch must handle 10,000 items in single transaction: {:?}",
        batch_res
    );

    // Verify job totals updated correctly in database
    let job = journal.get_job(&job_id).unwrap().expect("Job exists");
    assert_eq!(job.total_files, total_files as u64);
    assert_eq!(job.total_bytes, manifest.total_bytes);
}

// =========================================================================
// 4. 1 GiB APFS Sparse File Hole Preservation & BLAKE3 Digest Match (TEST-10)
// =========================================================================
#[test]
fn test_challenge_1gib_apfs_sparse_file_preservation() {
    let env = TestEnvironment::new();

    // 1 GiB logical size
    let logical_size = 1024 * 1024 * 1024u64; // 1 GiB

    // Extents: non-zero payload at offset 0 (header), middle (512 MiB), and near end (1 GiB - 4 KiB)
    let header_payload = b"SPARSE_CHALLENGE_HEADER_DATA_OFFSET_0";
    let middle_payload = b"SPARSE_CHALLENGE_MIDDLE_DATA_OFFSET_512MB";
    let tail_payload = b"SPARSE_CHALLENGE_TAIL_DATA_OFFSET_1GB";

    let extents = [
        (0u64, &header_payload[..]),
        (512 * 1024 * 1024u64, &middle_payload[..]),
        (logical_size - 4096, &tail_payload[..]),
    ];

    let mut builder = SyntheticDatasetBuilder::new(env.src_path());
    builder
        .add_sparse_file("large_sparse_1gib.img", logical_size, &extents)
        .expect("Create 1 GiB APFS sparse file");

    let src_file_path = env.src_path().join("large_sparse_1gib.img");
    let src_meta = fs::metadata(&src_file_path).unwrap();
    assert_eq!(src_meta.len(), logical_size);

    let src_blocks = src_meta.blocks();
    let src_allocated_bytes = src_blocks * 512;
    println!(
        "Source 1 GiB sparse file: logical_size={}, allocated_blocks={}, allocated_bytes={}",
        logical_size, src_blocks, src_allocated_bytes
    );
    // On APFS, a 1 GiB sparse file with ~100 bytes written should have very few blocks (< 1000 blocks = < 512 KiB)
    assert!(
        src_allocated_bytes < 10 * 1024 * 1024,
        "Source file must be sparse on APFS, allocated_bytes={}",
        src_allocated_bytes
    );

    let manifest = env.scan_source();
    let journal = env.open_journal();
    let job_id = "job_challenge_sparse_1gib".to_string();

    journal
        .create_job(&JobRecord {
            id: job_id.clone(),
            source_root: env.src_path().to_path_buf(),
            target_root: env.dst_path().to_path_buf(),
            source_volume_uuid: None,
            target_volume_uuid: None,
            mode: JobMode::SmartHybrid,
            state: JobState::Running,
            total_files: manifest.file_count,
            total_bytes: manifest.total_bytes,
            copied_bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
        })
        .unwrap();

    let file_records: Vec<_> = manifest
        .items
        .iter()
        .map(|item| FileRecord {
            id: item.id,
            job_id: job_id.clone(),
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
    journal.record_files_batch(&job_id, &file_records).unwrap();

    let recovery = Arc::new(RecoveryController::with_journal(
        Arc::new(volterra_recovery::ErrorQueue::new()),
        RetryBudget::default(),
        journal.clone(),
    ));

    let config = TransferSchedulerConfig {
        source_root: env.src_path().to_path_buf(),
        target_root: env.dst_path().to_path_buf(),
        job_id: job_id.clone(),
        chunk_size: 8 * 1024 * 1024, // 8 MiB chunks
        memory_budget_bytes: 32 * 1024 * 1024,
        direct_io: false,
        verify_readback: true,
        allow_overwrite: false,
    };

    let scheduler = TransferScheduler::new(
        config,
        journal,
        recovery,
        Arc::new(PerformanceMonitor::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );

    let res = scheduler.execute_pipeline(&manifest, |_, _, _, _| {});
    assert!(res.is_ok(), "Transfer of sparse file failed: {:?}", res);

    let dst_file_path = env.dst_path().join("large_sparse_1gib.img");
    assert!(dst_file_path.exists(), "Destination file must exist");

    let dst_meta = fs::metadata(&dst_file_path).unwrap();
    assert_eq!(
        dst_meta.len(),
        logical_size,
        "Destination file logical size must be 1 GiB"
    );

    let dst_blocks = dst_meta.blocks();
    let dst_allocated_bytes = dst_blocks * 512;
    println!(
        "Destination 1 GiB file: logical_size={}, allocated_blocks={}, allocated_bytes={}",
        logical_size, dst_blocks, dst_allocated_bytes
    );

    // Deep validation using SyntheticDatasetValidator (checks size, BLAKE3 digest, and sparse preservation)
    let val_res = SyntheticDatasetValidator::validate(env.dst_path(), builder.expectations());
    match val_res {
        Ok(summary) => {
            println!("Validation succeeded: {:?}", summary);
            assert_eq!(summary.files_matched, 1);
        }
        Err(err) => {
            panic!("Synthetic dataset validation failed:\n{}", err);
        }
    }
}
