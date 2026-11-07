//! Empirical stress test harness for `volterra-manifest`.
//!
//! Evaluates:
//! 1. Directory symlink cyclic traps (self-loop, parent loop, mutual cycle, root reference, dangling chain).
//! 2. Termination, cycle-breaking invariants, and heap safety under adversarial filesystem graphs.
//! 3. Pre-publication source mutation detection (append, truncation, mtime change, file deletion, open fd mutation).
//! 4. Catalog chunking boundary conditions.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::symlink;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::Instant;
use tempfile::tempdir;
use volterra_manifest::{
    ItemType, ManifestCatalog, ManifestError, ManifestScanner, MutationStatus, ScanOptions,
    SourceBaseline, SourceMutationDetector,
};
use volterra_platform_macos::{FileTimestamps, PlatformTime};

#[test]
fn test_symlink_trap_parent_circular_loop() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    // Construct hierarchy:
    // root/
    //   file_root.txt
    //   dir_a/
    //     file_a.txt
    //     loop_to_parent -> .. (points back to root)
    //     dir_b/
    //       file_b.txt
    //       loop_to_grandparent -> ../.. (points back to root)
    let dir_a = root.join("dir_a");
    let dir_b = dir_a.join("dir_b");
    fs::create_dir_all(&dir_b).expect("mkdir");

    fs::write(root.join("file_root.txt"), b"root").expect("write");
    fs::write(dir_a.join("file_a.txt"), b"a").expect("write");
    fs::write(dir_b.join("file_b.txt"), b"b").expect("write");

    symlink("..", dir_a.join("loop_to_parent")).expect("symlink parent");
    symlink("../..", dir_b.join("loop_to_grandparent")).expect("symlink grandparent");

    let start = Instant::now();
    let manifest = ManifestScanner::scan_tree(root, &ScanOptions::default()).expect("scan");
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 500,
        "Scanner must not loop infinitely; elapsed {:?}",
        elapsed
    );

    // Verify exactly 3 regular files, 2 directories, and 2 symlinks cataloged
    assert_eq!(manifest.file_count, 3, "Regular file count mismatch");
    assert_eq!(manifest.dir_count, 2, "Directory count mismatch");
    assert_eq!(manifest.symlink_count, 2, "Symlink count mismatch");

    // Ensure symlink items are preserved with correct targets
    let symlinks: Vec<_> = manifest
        .items
        .iter()
        .filter(|i| i.item_type == ItemType::Symlink)
        .collect();
    assert_eq!(symlinks.len(), 2);
    for s in symlinks {
        assert!(
            s.symlink_target.is_some(),
            "Symlink target must be recorded"
        );
    }
}

#[test]
fn test_symlink_trap_self_loop_and_dot_references() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    // Symlink pointing to itself: loop -> loop
    symlink("self_ref", root.join("self_ref")).expect("symlink self");
    // Symlink pointing to current dir: dot_ref -> .
    symlink(".", root.join("dot_ref")).expect("symlink dot");

    fs::write(root.join("payload.bin"), vec![0xAA; 1024]).expect("write");

    let start = Instant::now();
    let manifest = ManifestScanner::scan_tree(root, &ScanOptions::default()).expect("scan");
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 200,
        "Self loop must not stall scanner"
    );
    assert_eq!(manifest.file_count, 1);
    assert_eq!(manifest.symlink_count, 2);
}

#[test]
fn test_symlink_trap_mutual_cycle() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    // Construct mutual cross-reference:
    // root/
    //   group1/
    //     f1.txt
    //     link_to_g2 -> ../group2
    //   group2/
    //     f2.txt
    //     link_to_g1 -> ../group1
    let g1 = root.join("group1");
    let g2 = root.join("group2");
    fs::create_dir(&g1).expect("mkdir g1");
    fs::create_dir(&g2).expect("mkdir g2");

    fs::write(g1.join("f1.txt"), b"g1").expect("write");
    fs::write(g2.join("f2.txt"), b"g2").expect("write");

    symlink("../group2", g1.join("link_to_g2")).expect("symlink g2");
    symlink("../group1", g2.join("link_to_g1")).expect("symlink g1");

    let start = Instant::now();
    let manifest = ManifestScanner::scan_tree(root, &ScanOptions::default()).expect("scan");
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 200,
        "Mutual cycle must terminate promptly"
    );
    assert_eq!(manifest.file_count, 2);
    assert_eq!(manifest.dir_count, 2);
    assert_eq!(manifest.symlink_count, 2);
}

#[test]
fn test_symlink_trap_external_escape_prevention() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    // Symlink pointing to outside filesystem root: escape_root -> /
    // Must NOT follow and traverse the host root volume!
    symlink("/", root.join("escape_root")).expect("symlink root");
    symlink("/tmp", root.join("escape_tmp")).expect("symlink tmp");
    fs::write(root.join("local.txt"), b"local").expect("write");

    let start = Instant::now();
    let manifest = ManifestScanner::scan_tree(root, &ScanOptions::default()).expect("scan");
    let elapsed = start.elapsed();

    // If scanner followed / or /tmp, it would take seconds/minutes and find hundreds of thousands of files
    assert!(
        elapsed.as_millis() < 500,
        "Scanner must never follow symlinks escaping root; elapsed {:?}",
        elapsed
    );
    assert_eq!(manifest.file_count, 1, "Must not index outside files");
    assert_eq!(
        manifest.symlink_count, 2,
        "Must record 2 escape symlinks safely"
    );
}

#[test]
fn test_symlink_trap_dangling_and_chained() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    // Dangling symlink: points to nonexistent target
    symlink("nowhere.bin", root.join("dangling_link")).expect("symlink dangling");

    // Chain: chain1 -> chain2 -> chain3 -> dead_end
    symlink("chain2", root.join("chain1")).expect("symlink c1");
    symlink("chain3", root.join("chain2")).expect("symlink c2");
    symlink("dead_end", root.join("chain3")).expect("symlink c3");

    let manifest = ManifestScanner::scan_tree(root, &ScanOptions::default()).expect("scan");
    assert_eq!(manifest.symlink_count, 4);
    assert_eq!(manifest.file_count, 0);
}

#[test]
fn test_source_mutation_append_detection() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("append_target.bin");
    fs::write(&file_path, vec![0x11; 4096]).expect("write initial");

    let manifest = ManifestScanner::scan_tree(dir.path(), &ScanOptions::default()).expect("scan");
    let item = manifest
        .items
        .iter()
        .find(|i| i.relative_path == Path::new("append_target.bin"))
        .expect("find item");
    let baseline = SourceBaseline::from_item(item);

    // Append 1 byte to file
    let mut f = OpenOptions::new()
        .append(true)
        .open(&file_path)
        .expect("open append");
    f.write_all(&[0x99]).expect("write 1 byte");
    drop(f);

    // 1. revalidate_source_mutation
    let status =
        SourceMutationDetector::revalidate_source_mutation(item, &file_path).expect("revalidate");
    assert_eq!(
        status,
        MutationStatus::SizeChanged {
            original: 4096,
            current: 4097,
        }
    );

    // 2. check_unmodified
    let res = SourceMutationDetector::check_unmodified(&file_path, &baseline);
    assert!(
        matches!(res, Err(ManifestError::SourceMutated { .. })),
        "check_unmodified must fail on appended source"
    );

    // 3. check_fd_unmodified
    let fd_file = File::open(&file_path).expect("open fd");
    let fd_res =
        SourceMutationDetector::check_fd_unmodified(fd_file.as_raw_fd(), &file_path, &baseline);
    assert!(
        matches!(fd_res, Err(ManifestError::SourceMutated { .. })),
        "check_fd_unmodified must fail on appended source"
    );
}

#[test]
fn test_source_mutation_truncation_detection() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("truncate_target.bin");
    fs::write(&file_path, vec![0x22; 8192]).expect("write initial");

    let manifest = ManifestScanner::scan_tree(dir.path(), &ScanOptions::default()).expect("scan");
    let item = manifest
        .items
        .iter()
        .find(|i| i.relative_path == Path::new("truncate_target.bin"))
        .expect("find item");
    let baseline = SourceBaseline::from_item(item);

    // Truncate file from 8192 to 4096 bytes
    let f = OpenOptions::new()
        .write(true)
        .open(&file_path)
        .expect("open write");
    f.set_len(4096).expect("truncate");
    drop(f);

    // Verify detection
    let status =
        SourceMutationDetector::revalidate_source_mutation(item, &file_path).expect("revalidate");
    assert_eq!(
        status,
        MutationStatus::SizeChanged {
            original: 8192,
            current: 4096,
        }
    );

    let res = SourceMutationDetector::check_unmodified(&file_path, &baseline);
    assert!(matches!(res, Err(ManifestError::SourceMutated { .. })));
}

#[test]
fn test_source_mutation_mtime_advancement_detection() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("mtime_target.bin");
    fs::write(&file_path, b"constant payload").expect("write");

    let manifest = ManifestScanner::scan_tree(dir.path(), &ScanOptions::default()).expect("scan");
    let item = manifest
        .items
        .iter()
        .find(|i| i.relative_path == Path::new("mtime_target.bin"))
        .expect("find item");
    let baseline = SourceBaseline::from_item(item);

    // Advance mtime by +3600 seconds (1 hour)
    let updated_ts = FileTimestamps {
        btime_sec: item.mtime_sec,
        btime_nsec: 0,
        mtime_sec: item.mtime_sec + 3600,
        mtime_nsec: 0,
    };
    PlatformTime::set_timestamps(&file_path, &updated_ts).expect("set_timestamps");

    // Verify detection
    let status =
        SourceMutationDetector::revalidate_source_mutation(item, &file_path).expect("revalidate");
    assert!(
        matches!(status, MutationStatus::MtimeChanged { original_sec, current_sec, .. } if original_sec == item.mtime_sec && current_sec == item.mtime_sec + 3600),
        "Expected MtimeChanged, got {:?}",
        status
    );

    let res = SourceMutationDetector::check_unmodified(&file_path, &baseline);
    assert!(matches!(res, Err(ManifestError::SourceMutated { .. })));

    let fd_file = File::open(&file_path).expect("open fd");
    let fd_res =
        SourceMutationDetector::check_fd_unmodified(fd_file.as_raw_fd(), &file_path, &baseline);
    assert!(matches!(fd_res, Err(ManifestError::SourceMutated { .. })));
}

#[test]
fn test_source_mutation_deletion_detection() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("unlink_target.bin");
    fs::write(&file_path, b"will be deleted").expect("write");

    let manifest = ManifestScanner::scan_tree(dir.path(), &ScanOptions::default()).expect("scan");
    let item = manifest
        .items
        .iter()
        .find(|i| i.relative_path == Path::new("unlink_target.bin"))
        .expect("find item");
    let baseline = SourceBaseline::from_item(item);

    // Delete file
    fs::remove_file(&file_path).expect("remove_file");

    let status =
        SourceMutationDetector::revalidate_source_mutation(item, &file_path).expect("revalidate");
    assert_eq!(status, MutationStatus::Deleted);

    let res = SourceMutationDetector::check_unmodified(&file_path, &baseline);
    assert!(
        matches!(res, Err(ManifestError::SourceDeleted { .. })),
        "check_unmodified must return SourceDeleted"
    );
}

#[test]
fn test_catalog_chunking_boundaries() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    const CHUNK_SIZE: usize = 8 * 1024 * 1024; // 8 MiB

    // 0 bytes -> 1 zero-length chunk with canonical BLAKE3 empty hash
    let f0 = root.join("zero.bin");
    File::create(&f0).expect("create f0");

    // 1 byte -> 1 chunk of length 1
    let f1 = root.join("one.bin");
    fs::write(&f1, [0x42]).expect("write f1");

    // 8 MiB - 1 -> 1 chunk of length 8388607
    let f_under = root.join("under.bin");
    let file_under = File::create(&f_under).expect("create f_under");
    file_under
        .set_len((CHUNK_SIZE - 1) as u64)
        .expect("set_len");

    // 8 MiB -> 1 chunk of length 8388608
    let f_exact = root.join("exact.bin");
    let file_exact = File::create(&f_exact).expect("create f_exact");
    file_exact.set_len(CHUNK_SIZE as u64).expect("set_len");

    // 8 MiB + 1 -> 2 chunks: chunk 0 = 8 MiB, chunk 1 = 1 byte
    let f_over = root.join("over.bin");
    let file_over = File::create(&f_over).expect("create f_over");
    file_over.set_len((CHUNK_SIZE + 1) as u64).expect("set_len");

    // 16 MiB -> 2 chunks: chunk 0 = 8 MiB, chunk 1 = 8 MiB
    let f_16m = root.join("16m.bin");
    let file_16m = File::create(&f_16m).expect("create f_16m");
    file_16m.set_len((2 * CHUNK_SIZE) as u64).expect("set_len");

    let manifest = ManifestScanner::scan_tree(root, &ScanOptions::default()).expect("scan");

    // Check item chunk generation
    let item_0 = ManifestCatalog::find_by_path(&manifest, Path::new("zero.bin")).unwrap();
    let chunks_0 = ManifestCatalog::generate_chunks_for_file(item_0, CHUNK_SIZE);
    assert_eq!(chunks_0.len(), 1);
    assert_eq!(chunks_0[0].offset_bytes, 0);
    assert_eq!(chunks_0[0].length_bytes, 0);
    assert_eq!(
        chunks_0[0].digest.as_deref(),
        Some("af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262")
    );

    let item_1 = ManifestCatalog::find_by_path(&manifest, Path::new("one.bin")).unwrap();
    let chunks_1 = ManifestCatalog::generate_chunks_for_file(item_1, CHUNK_SIZE);
    assert_eq!(chunks_1.len(), 1);
    assert_eq!(chunks_1[0].length_bytes, 1);

    let item_under = ManifestCatalog::find_by_path(&manifest, Path::new("under.bin")).unwrap();
    let chunks_under = ManifestCatalog::generate_chunks_for_file(item_under, CHUNK_SIZE);
    assert_eq!(chunks_under.len(), 1);
    assert_eq!(chunks_under[0].length_bytes, (CHUNK_SIZE - 1) as u32);

    let item_exact = ManifestCatalog::find_by_path(&manifest, Path::new("exact.bin")).unwrap();
    let chunks_exact = ManifestCatalog::generate_chunks_for_file(item_exact, CHUNK_SIZE);
    assert_eq!(chunks_exact.len(), 1);
    assert_eq!(chunks_exact[0].length_bytes, CHUNK_SIZE as u32);

    let item_over = ManifestCatalog::find_by_path(&manifest, Path::new("over.bin")).unwrap();
    let chunks_over = ManifestCatalog::generate_chunks_for_file(item_over, CHUNK_SIZE);
    assert_eq!(chunks_over.len(), 2);
    assert_eq!(chunks_over[0].offset_bytes, 0);
    assert_eq!(chunks_over[0].length_bytes, CHUNK_SIZE as u32);
    assert_eq!(chunks_over[1].offset_bytes, CHUNK_SIZE as u64);
    assert_eq!(chunks_over[1].length_bytes, 1);

    let item_16m = ManifestCatalog::find_by_path(&manifest, Path::new("16m.bin")).unwrap();
    let chunks_16m = ManifestCatalog::generate_chunks_for_file(item_16m, CHUNK_SIZE);
    assert_eq!(chunks_16m.len(), 2);
    assert_eq!(chunks_16m[0].length_bytes, CHUNK_SIZE as u32);
    assert_eq!(chunks_16m[1].length_bytes, CHUNK_SIZE as u32);

    // Total chunks: 1 (zero) + 1 (one) + 1 (under) + 1 (exact) + 2 (over) + 2 (16m) = 8 chunks
    let total = ManifestCatalog::compute_total_chunks(&manifest, CHUNK_SIZE);
    assert_eq!(total, 8, "Total chunks across boundary files mismatch");
}

#[test]
fn test_manifest_scanner_deep_tree_no_stack_overflow() {
    let dir = tempdir().expect("tempdir");
    let mut current_dir = dir.path().to_path_buf();

    // Create a 30-level deep nesting hierarchy
    const DEPTH: usize = 30;
    for d in 0..DEPTH {
        current_dir = current_dir.join(format!("depth_{}", d));
    }
    fs::create_dir_all(&current_dir).expect("create_dir_all deep");

    // Put a file at the deepest level
    let leaf_file = current_dir.join("deep_leaf.txt");
    fs::write(&leaf_file, b"deep leaf content").expect("write leaf");

    let start = Instant::now();
    let manifest =
        ManifestScanner::scan_tree(dir.path(), &ScanOptions::default()).expect("scan deep");
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 500,
        "Deep scan must terminate quickly"
    );
    assert_eq!(manifest.dir_count, DEPTH as u64);
    assert_eq!(manifest.file_count, 1);

    let leaf_item = manifest
        .items
        .iter()
        .find(|i| i.item_type == ItemType::RegularFile)
        .expect("must find leaf file");
    assert_eq!(leaf_item.size_bytes, 17);
}
