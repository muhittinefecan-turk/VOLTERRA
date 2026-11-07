use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use volterra_manifest::{
    FileState, ItemType, Manifest, ManifestCatalog, ManifestError, ManifestItem, ManifestScanner,
    MutationStatus, ScanOptions, SourceBaseline, SourceValidator,
};
use volterra_platform_macos::{FileTimestamps, PlatformTime};

#[test]
fn test_scan_tree_nested_structure() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    // Create 4 nested directories: level1, level1/level2, level1/level2/level3, level1/level2/level3/level4
    let d1 = root.join("level1");
    let d2 = d1.join("level2");
    let d3 = d2.join("level3");
    let d4 = d3.join("level4");
    fs::create_dir_all(&d4).expect("create_dir_all");

    // Distribute 20 files across root and the 4 subdirectories (4 files each)
    let dirs = [root, d1.as_path(), d2.as_path(), d3.as_path(), d4.as_path()];
    let mut file_idx = 1;
    for d in dirs {
        for _ in 0..4 {
            let file_path = d.join(format!("file_{}.txt", file_idx));
            fs::write(&file_path, format!("Content of file {}", file_idx)).expect("write");
            file_idx += 1;
        }
    }

    let manifest = ManifestScanner::scan_tree(root, &ScanOptions::default()).expect("scan_tree");

    assert_eq!(
        manifest.dir_count, 4,
        "Must detect exactly 4 subdirectories"
    );
    assert_eq!(manifest.file_count, 20, "Must detect exactly 20 files");

    // Verify all relative paths have no leading slashes
    for item in &manifest.items {
        let path_str = item.relative_path.to_string_lossy();
        assert!(
            !path_str.starts_with('/'),
            "Relative path '{}' must not have leading slash",
            path_str
        );
    }
}

#[test]
fn test_scan_tree_zero_byte_file() {
    let dir = tempdir().expect("tempdir");
    let empty_file = dir.path().join("empty.bin");
    File::create(&empty_file).expect("create empty file");

    let manifest = ManifestScanner::scan_tree(dir.path(), &ScanOptions::default()).expect("scan");
    let item = manifest
        .items
        .iter()
        .find(|i| i.relative_path == Path::new("empty.bin"))
        .expect("empty.bin must be in manifest");

    assert_eq!(item.size_bytes, 0);
    assert_eq!(item.source_size, 0);
    assert_eq!(item.item_type, ItemType::RegularFile);

    // Verify chunk generation on zero-byte file produces 1 empty chunk with canonical BLAKE3 hash
    let chunks = ManifestCatalog::generate_chunks_for_file(item, 8 * 1024 * 1024);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].length_bytes, 0);
    let dynamic_empty_digest = volterra_integrity::Blake3Hasher::hash_to_digest(b"").to_hex();
    assert_eq!(
        chunks[0].digest.as_deref(),
        Some(dynamic_empty_digest.as_str())
    );
    assert_eq!(
        chunks[0].digest.as_deref(),
        Some("af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262")
    );
}

#[test]
fn test_scan_tree_symlink_no_recurse() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    let sub_dir = root.join("sub");
    fs::create_dir(&sub_dir).expect("create sub");
    fs::write(sub_dir.join("inside.txt"), b"inside").expect("write inside");

    // Create cyclic symlink: sub/loop_to_parent pointing to ..
    let cycle_link = sub_dir.join("loop_to_parent");
    std::os::unix::fs::symlink("..", &cycle_link).expect("create cycle link");

    let manifest = ManifestScanner::scan_tree(root, &ScanOptions::default()).expect("scan_tree");

    assert!(manifest.symlink_count >= 1);
    let link_item = manifest
        .items
        .iter()
        .find(|i| i.item_type == ItemType::Symlink)
        .expect("Symlink must be cataloged");

    assert_eq!(link_item.item_type, ItemType::Symlink);
    assert!(link_item.symlink_target.is_some());
    // Invariant: Scanner terminates promptly without recursion cycle
}

#[test]
fn test_manifest_catalog_totals() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    fs::write(root.join("f1.bin"), vec![0u8; 1000]).expect("write");
    fs::write(root.join("f2.bin"), vec![1u8; 2000]).expect("write");
    fs::write(root.join("f3.bin"), vec![2u8; 3000]).expect("write");

    let manifest = ManifestScanner::scan_tree(root, &ScanOptions::default()).expect("scan");

    let manual_sum: u64 = manifest
        .items
        .iter()
        .filter(|i| i.item_type == ItemType::RegularFile)
        .map(|i| i.size_bytes)
        .sum();

    assert_eq!(manifest.total_bytes, 6000);
    assert_eq!(manifest.total_bytes, manual_sum);
}

#[test]
fn test_manifest_catalog_hdd_ordering() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    let da = root.join("dir_a");
    let db = root.join("dir_b");
    fs::create_dir_all(&da).expect("mkdir");
    fs::create_dir_all(&db).expect("mkdir");

    fs::write(da.join("a1.txt"), b"1").expect("write");
    fs::write(db.join("b1.txt"), b"2").expect("write");
    fs::write(da.join("a2.txt"), b"3").expect("write");

    let manifest = ManifestScanner::scan_tree(root, &ScanOptions::default()).expect("scan");
    let hdd_order = ManifestCatalog::order_for_hdd(&manifest);

    assert_eq!(hdd_order.len(), manifest.items.len());

    // Verify parent directories are grouped together
    let mut last_parent: Option<PathBuf> = None;
    let mut parent_changes = 0;

    for &idx in &hdd_order {
        let parent = manifest.items[idx]
            .relative_path
            .parent()
            .map(|p| p.to_path_buf());
        if parent != last_parent {
            parent_changes += 1;
            last_parent = parent;
        }
    }

    // Since parent dirs are sorted together, parent changes must be <= total distinct parents
    assert!(parent_changes <= 3);
}

#[test]
fn test_manifest_catalog_ssd_ordering() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    fs::write(root.join("small.bin"), vec![0u8; 100]).expect("write");
    fs::write(root.join("medium.bin"), vec![0u8; 500]).expect("write");
    fs::write(root.join("tiny.bin"), vec![0u8; 50]).expect("write");
    fs::write(root.join("large.bin"), vec![0u8; 1000]).expect("write");

    let manifest = ManifestScanner::scan_tree(root, &ScanOptions::default()).expect("scan");
    let ssd_order = ManifestCatalog::order_for_ssd(&manifest);

    // Verify descending size order
    let mut prev_size = u64::MAX;
    for &idx in &ssd_order {
        let item = &manifest.items[idx];
        if item.item_type == ItemType::RegularFile {
            assert!(
                item.size_bytes <= prev_size,
                "SSD order must sort sizes descending: current {} > prev {}",
                item.size_bytes,
                prev_size
            );
            prev_size = item.size_bytes;
        }
    }
}

#[test]
fn test_source_access_readonly_check() {
    let dir = tempdir().expect("tempdir");
    let report =
        SourceValidator::validate_source_access(dir.path()).expect("validate_source_access failed");

    assert!(report.root_accessible);
}

#[test]
fn test_source_mutation_size_change() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("mutable.txt");
    fs::write(&file_path, b"Initial data").expect("write");

    let manifest = ManifestScanner::scan_tree(dir.path(), &ScanOptions::default()).expect("scan");
    let item = manifest
        .items
        .iter()
        .find(|i| i.relative_path == Path::new("mutable.txt"))
        .unwrap();

    // Mutate file size by appending 100 bytes
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(&file_path)
        .expect("open");
    f.write_all(&[0x42; 100]).expect("append");
    drop(f);

    let status =
        SourceValidator::revalidate_source_mutation(item, &file_path).expect("revalidate failed");

    assert!(
        matches!(status, MutationStatus::SizeChanged { original, current } if original == 12 && current == 112),
        "Expected SizeChanged mutation status, got {:?}",
        status
    );
}

#[test]
fn test_source_mutation_mtime_change() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("mtime_test.txt");
    fs::write(&file_path, b"mtime content").expect("write");

    let manifest = ManifestScanner::scan_tree(dir.path(), &ScanOptions::default()).expect("scan");
    let item = manifest
        .items
        .iter()
        .find(|i| i.relative_path == Path::new("mtime_test.txt"))
        .unwrap();

    // Change modification time via PlatformTime
    let updated_ts = FileTimestamps {
        btime_sec: item.mtime_sec,
        btime_nsec: 0,
        mtime_sec: item.mtime_sec + 500,
        mtime_nsec: 0,
    };
    PlatformTime::set_timestamps(&file_path, &updated_ts).expect("set_timestamps");

    let status =
        SourceValidator::revalidate_source_mutation(item, &file_path).expect("revalidate failed");

    assert!(
        matches!(status, MutationStatus::MtimeChanged { .. }),
        "Expected MtimeChanged, got {:?}",
        status
    );
}

#[test]
fn test_source_mutation_deleted_file() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("to_delete.txt");
    fs::write(&file_path, b"delete me").expect("write");

    let manifest = ManifestScanner::scan_tree(dir.path(), &ScanOptions::default()).expect("scan");
    let item = manifest
        .items
        .iter()
        .find(|i| i.relative_path == Path::new("to_delete.txt"))
        .unwrap();

    fs::remove_file(&file_path).expect("delete");

    let status =
        SourceValidator::revalidate_source_mutation(item, &file_path).expect("revalidate failed");

    assert_eq!(
        status,
        MutationStatus::Deleted,
        "Expected MutationStatus::Deleted when file removed"
    );
}

#[test]
fn test_unicode_and_turkish_paths() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    let turkish_name = "Türkçe_özet_şğüöı.txt";
    let emoji_name = "🚀.data";

    let f1 = root.join(turkish_name);
    let f2 = root.join(emoji_name);

    fs::write(&f1, b"Turkish characters verified").expect("write turkish");
    fs::write(&f2, b"Emoji verified").expect("write emoji");

    let manifest = ManifestScanner::scan_tree(root, &ScanOptions::default()).expect("scan");

    let found_turkish = manifest.items.iter().any(|i| {
        // macOS APFS normalizes unicode with NFD/NFC; compare string equality or contained
        let name = i.relative_path.to_string_lossy();
        name.contains("özet") && name.contains(".txt")
    });
    assert!(found_turkish, "Turkish unicode filename must be preserved");

    let found_emoji = manifest.items.iter().any(|i| {
        let name = i.relative_path.to_string_lossy();
        name.contains("🚀")
    });
    assert!(
        found_emoji,
        "Emoji filename must be preserved without byte mangling"
    );
}

#[test]
fn test_catalog_chunk_size_zero_safety() {
    let item_empty = ManifestItem {
        id: 1,
        job_id: "job_test".to_string(),
        relative_path: PathBuf::from("empty.bin"),
        item_type: ItemType::RegularFile,
        size_bytes: 0,
        source_size: 0,
        mtime_sec: 0,
        mtime_nsec: 0,
        source_mtime_ns: 0,
        btime_nsec: None,
        source_btime_ns: None,
        inode: 1,
        dev: 1,
        source_file_id: "1:1".to_string(),
        posix_mode: 0o644,
        has_xattrs: false,
        symlink_target: None,
        temp_path: None,
        state: FileState::Pending,
        verified_digest: None,
        error_code: None,
    };
    let item_data = ManifestItem {
        id: 2,
        job_id: "job_test".to_string(),
        relative_path: PathBuf::from("data.bin"),
        item_type: ItemType::RegularFile,
        size_bytes: 1024 * 1024,
        source_size: 1024 * 1024,
        mtime_sec: 0,
        mtime_nsec: 0,
        source_mtime_ns: 0,
        btime_nsec: None,
        source_btime_ns: None,
        inode: 2,
        dev: 1,
        source_file_id: "1:2".to_string(),
        posix_mode: 0o644,
        has_xattrs: false,
        symlink_target: None,
        temp_path: None,
        state: FileState::Pending,
        verified_digest: None,
        error_code: None,
    };

    // Neither call may panic with divide-by-zero
    let chunks_empty = ManifestCatalog::generate_chunks_for_file(&item_empty, 0);
    assert!(
        chunks_empty.is_empty(),
        "chunk_size == 0 must return empty chunks"
    );

    let chunks_data = ManifestCatalog::generate_chunks_for_file(&item_data, 0);
    assert!(
        chunks_data.is_empty(),
        "chunk_size == 0 must return empty chunks"
    );

    let manifest = Manifest {
        root: PathBuf::from("/test"),
        job_id: "job_test".to_string(),
        items: vec![item_empty, item_data],
        total_bytes: 1024 * 1024,
        file_count: 2,
        dir_count: 0,
        symlink_count: 0,
        special_count: 0,
    };
    let total = ManifestCatalog::compute_total_chunks(&manifest, 0);
    assert_eq!(
        total, 0,
        "compute_total_chunks with chunk_size == 0 must return 0 without panicking"
    );
}

#[test]
fn test_scan_tree_isolated_subdirectory_permission_denied() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();

    // 1. Create readable subdirectory with file
    let readable_dir = root.join("readable_dir");
    fs::create_dir(&readable_dir).expect("create readable_dir");
    fs::write(readable_dir.join("healthy.txt"), b"healthy data").expect("write healthy");

    // 2. Create restricted subdirectory (000 permissions)
    let restricted_dir = root.join("restricted_dir");
    fs::create_dir(&restricted_dir).expect("create restricted_dir");
    fs::write(restricted_dir.join("secret.txt"), b"secret data").expect("write secret");
    fs::set_permissions(&restricted_dir, fs::Permissions::from_mode(0o000)).expect("chmod 000");

    // 3. Create another sibling readable subdirectory with file
    let readable_dir_2 = root.join("readable_dir_2");
    fs::create_dir(&readable_dir_2).expect("create readable_dir_2");
    fs::write(readable_dir_2.join("healthy_2.txt"), b"healthy data 2").expect("write healthy 2");

    // Scan must succeed rather than aborting on restricted_dir
    let scan_res = ManifestScanner::scan_tree(root, &ScanOptions::default());

    // Restore permissions immediately so tempdir drop cleanup succeeds on macOS
    let _ = fs::set_permissions(&restricted_dir, fs::Permissions::from_mode(0o755));

    let manifest = scan_res.expect("scan_tree must not abort on restricted subdirectory");

    // Verify healthy files are present
    assert!(manifest
        .items
        .iter()
        .any(|i| i.relative_path == Path::new("readable_dir/healthy.txt")
            && i.state == FileState::Pending));
    assert!(manifest.items.iter().any(|i| i.relative_path
        == Path::new("readable_dir_2/healthy_2.txt")
        && i.state == FileState::Pending));

    // Verify restricted directory is quarantined with NeedsAttention and EACCES
    let restricted_item = manifest
        .items
        .iter()
        .find(|i| i.relative_path == Path::new("restricted_dir"))
        .expect("restricted_dir must be cataloged");
    assert_eq!(restricted_item.state, FileState::NeedsAttention);
    assert_eq!(restricted_item.error_code, Some(libc::EACCES));
}

#[test]
fn test_scan_tree_root_permission_denied_fails_fast() {
    let dir = tempdir().expect("tempdir");
    let restricted_root = dir.path().join("restricted_root");
    fs::create_dir(&restricted_root).expect("create restricted_root");
    fs::write(restricted_root.join("data.txt"), b"data").expect("write");
    fs::set_permissions(&restricted_root, fs::Permissions::from_mode(0o000)).expect("chmod 000");

    let scan_res = ManifestScanner::scan_tree(&restricted_root, &ScanOptions::default());

    // Restore permissions immediately for tempdir cleanup
    let _ = fs::set_permissions(&restricted_root, fs::Permissions::from_mode(0o755));

    match scan_res {
        Err(ManifestError::PermissionDenied(path)) => {
            assert!(path.ends_with("restricted_root"));
        }
        other => panic!("Expected Err(PermissionDenied), got {:?}", other),
    }
}

#[test]
fn test_source_mutation_inode_replacement_caught() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("replaced.txt");
    fs::write(&file_path, b"Original file contents").expect("write");

    let manifest = ManifestScanner::scan_tree(dir.path(), &ScanOptions::default()).expect("scan");
    let item = manifest
        .items
        .iter()
        .find(|i| i.relative_path == Path::new("replaced.txt"))
        .expect("find item");
    let baseline = SourceBaseline::from_item(item);

    // Delete and recreate file with identical size and content
    fs::remove_file(&file_path).expect("remove");
    fs::write(&file_path, b"Original file contents").expect("recreate");

    // 1. revalidate_source_mutation must catch Inode IdentityChanged
    let status = SourceValidator::revalidate_source_mutation(item, &file_path).expect("revalidate");
    match status {
        MutationStatus::IdentityChanged {
            expected_ino,
            actual_ino,
        } => {
            assert_eq!(expected_ino, item.inode);
            assert_ne!(actual_ino, item.inode);
        }
        other => panic!("Expected IdentityChanged, got {:?}", other),
    }

    // 2. check_unmodified must fail with identity error
    let res = SourceValidator::check_unmodified(&file_path, &baseline);
    assert!(
        matches!(res, Err(ManifestError::SourceMutated { reason, .. }) if reason.contains("identity changed")),
        "check_unmodified must detect inode swap"
    );

    // 3. check_fd_unmodified with open fd to newly created file must fail against original baseline
    let fd_file = File::open(&file_path).expect("open");
    let fd_res = SourceValidator::check_fd_unmodified(fd_file.as_raw_fd(), &file_path, &baseline);
    assert!(
        matches!(fd_res, Err(ManifestError::SourceMutated { reason, .. }) if reason.contains("identity mismatch")),
        "check_fd_unmodified must detect swapped fd"
    );
}

#[test]
fn test_source_mutation_subsecond_nanoseconds_caught() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("nano_test.txt");
    fs::write(&file_path, b"test nanosecond mutation").expect("write");

    let manifest = ManifestScanner::scan_tree(dir.path(), &ScanOptions::default()).expect("scan");
    let item = manifest
        .items
        .iter()
        .find(|i| i.relative_path == Path::new("nano_test.txt"))
        .expect("find item");
    let baseline = SourceBaseline::from_item(item);

    // Preserve identical seconds, advance nanoseconds by +500,000 ns
    let updated_ts = FileTimestamps {
        btime_sec: item.mtime_sec,
        btime_nsec: item.btime_nsec.unwrap_or(0),
        mtime_sec: item.mtime_sec,
        mtime_nsec: item.mtime_nsec + 500_000,
    };
    PlatformTime::set_timestamps(&file_path, &updated_ts).expect("set_timestamps");

    // 1. revalidate_source_mutation must catch subsecond change
    let status = SourceValidator::revalidate_source_mutation(item, &file_path).expect("revalidate");
    match status {
        MutationStatus::MtimeChanged {
            original_sec,
            current_sec,
            original_nsec,
            current_nsec,
        } => {
            assert_eq!(
                original_sec, current_sec,
                "Seconds should remain unchanged in intra-second test"
            );
            assert_ne!(
                original_nsec, current_nsec,
                "Nanoseconds must detect intra-second modification"
            );
        }
        other => panic!(
            "Expected MtimeChanged with subsecond difference, got {:?}",
            other
        ),
    }

    // 2. check_unmodified must fail
    let res = SourceValidator::check_unmodified(&file_path, &baseline);
    assert!(
        matches!(res, Err(ManifestError::SourceMutated { reason, .. }) if reason.contains("Modification time changed")),
        "check_unmodified must detect subsecond change"
    );

    // 3. check_fd_unmodified must fail
    let fd_file = File::open(&file_path).expect("open");
    let fd_res = SourceValidator::check_fd_unmodified(fd_file.as_raw_fd(), &file_path, &baseline);
    assert!(
        matches!(fd_res, Err(ManifestError::SourceMutated { reason, .. }) if reason.contains("mtime mismatch")),
        "check_fd_unmodified must detect subsecond fd change"
    );
}
