use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::symlink;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use tempfile::tempdir;
use volterra_platform_macos::{
    AtomicReplaceResult, FileTimestamps, PlatformAtomic, PlatformError, PlatformIo,
    PlatformMetadata, PlatformSymlink, PlatformTime, PlatformXattr,
};

// ============================================================================
// AREA 1: ATOMIC REPLACEMENT & COLLISION BLOCKING
// ============================================================================

#[test]
fn test_adversarial_atomic_collision_preserves_target_and_partial() {
    let dir = tempdir().expect("Failed to create tempdir");
    let target_dest = dir.path().join("critical_target.dat");
    let source_partial = dir
        .path()
        .join(".volterra.job99.critical_target.dat.partial");

    let original_payload = b"CRITICAL_USER_DATA_DO_NOT_CORRUPT";
    fs::write(&target_dest, original_payload).expect("Failed to write target");

    let new_payload = b"NEW_UNVERIFIED_DATA";
    fs::write(&source_partial, new_payload).expect("Failed to write partial");

    // Attempt atomic replace with allow_overwrite = false
    let res = PlatformAtomic::atomic_replace(&source_partial, &target_dest, false);

    // 1. MUST return TargetExists error
    assert!(
        matches!(res, Err(PlatformError::TargetExists(_))),
        "atomic_replace must fail with TargetExists when overwrite is disallowed, got {:?}",
        res
    );

    // 2. Target file MUST be untouched
    assert_eq!(
        fs::read(&target_dest).expect("Failed to read target"),
        original_payload,
        "Target file was modified despite overwrite disallowed!"
    );

    // 3. Source partial file MUST remain intact on disk for resume/recovery
    assert!(
        source_partial.exists(),
        "Partial file was deleted or unlinked on collision failure!"
    );
    assert_eq!(
        fs::read(&source_partial).expect("Failed to read partial"),
        new_payload,
        "Partial file data was corrupted on collision!"
    );
}

#[test]
fn test_adversarial_atomic_collision_multithreaded_race() {
    let dir = tempdir().expect("Failed to create tempdir");
    let shared_target = Arc::new(dir.path().join("raced_target.dat"));

    let num_threads = 16;
    let barrier = Arc::new(Barrier::new(num_threads));

    let mut handles = Vec::new();
    for i in 0..num_threads {
        let target = Arc::clone(&shared_target);
        let b = Arc::clone(&barrier);
        let partial_path = dir
            .path()
            .join(format!(".volterra.job_thread_{}.partial", i));
        let thread_payload = format!("THREAD_{}_PAYLOAD", i).into_bytes();
        fs::write(&partial_path, &thread_payload).expect("write partial");

        handles.push(thread::spawn(move || {
            b.wait();
            let res = PlatformAtomic::atomic_replace(&partial_path, &target, false);
            (i, res, partial_path, thread_payload)
        }));
    }

    let mut successes = 0;
    let mut collisions = 0;
    let mut winning_payload = Vec::new();

    for h in handles {
        let (idx, res, partial_path, payload) = h.join().expect("thread panic");
        match res {
            Ok(AtomicReplaceResult::PublishedNew) => {
                successes += 1;
                winning_payload = payload;
                assert!(
                    !partial_path.exists(),
                    "Winning partial file should have been published"
                );
            }
            Err(PlatformError::TargetExists(_)) => {
                collisions += 1;
                assert!(
                    partial_path.exists(),
                    "Losing partial {} must be preserved intact",
                    idx
                );
            }
            other => panic!("Unexpected result from race: {:?}", other),
        }
    }

    // Exactly one thread must succeed in creating the new file
    assert_eq!(
        successes, 1,
        "Expected exactly 1 thread to succeed, got {}",
        successes
    );
    assert_eq!(
        collisions,
        num_threads - 1,
        "Expected {} threads to encounter collision, got {}",
        num_threads - 1,
        collisions
    );

    // Destination content must match the single winning thread's payload
    let target_content = fs::read(&*shared_target).expect("read shared target");
    assert_eq!(target_content, winning_payload);
}

#[test]
fn test_adversarial_atomic_collision_against_existing_multithreaded() {
    let dir = tempdir().expect("Failed to create tempdir");
    let shared_target = Arc::new(dir.path().join("preexisting_target.dat"));
    let initial_data = b"PRE_EXISTING_GOLDEN_DATA";
    fs::write(&*shared_target, initial_data).expect("write initial");

    let num_threads = 12;
    let barrier = Arc::new(Barrier::new(num_threads));

    let mut handles = Vec::new();
    for i in 0..num_threads {
        let target = Arc::clone(&shared_target);
        let b = Arc::clone(&barrier);
        let partial_path = dir
            .path()
            .join(format!(".volterra.contender_{}.partial", i));
        fs::write(&partial_path, format!("CONTENDER_{}", i)).expect("write");

        handles.push(thread::spawn(move || {
            b.wait();
            let res = PlatformAtomic::atomic_replace(&partial_path, &target, false);
            (i, res, partial_path)
        }));
    }

    for h in handles {
        let (i, res, partial_path) = h.join().expect("thread join");
        assert!(
            matches!(res, Err(PlatformError::TargetExists(_))),
            "Thread {} should have failed with TargetExists, got {:?}",
            i,
            res
        );
        assert!(
            partial_path.exists(),
            "Thread {} partial must still exist",
            i
        );
    }

    assert_eq!(
        fs::read(&*shared_target).expect("read target"),
        initial_data,
        "Target was modified despite overwrite disallowed across all threads"
    );
}

#[test]
fn test_adversarial_atomic_collision_with_dangling_symlink() {
    let dir = tempdir().expect("Failed to create tempdir");
    let dangling_target = dir.path().join("dangling_target.dat");
    let missing_dest = PathBuf::from("does_not_exist_anywhere.txt");

    // Create dangling symlink at target destination
    symlink(&missing_dest, &dangling_target).expect("create dangling symlink");

    let partial_path = dir.path().join("data.partial");
    fs::write(&partial_path, b"SOME NEW DATA").expect("write partial");

    // Attempt atomic replace with allow_overwrite = false
    let res = PlatformAtomic::atomic_replace(&partial_path, &dangling_target, false);

    // Darwin renamex_np with RENAME_EXCL on a symlink target returns EEXIST (or TargetExists)
    // Destination symlink must NOT be silently replaced without overwrite approval
    assert!(
        res.is_err(),
        "atomic_replace must not overwrite dangling symlink without allow_overwrite = true"
    );
    assert!(
        PlatformSymlink::is_symlink(&dangling_target),
        "Dangling symlink was destroyed!"
    );
    assert!(partial_path.exists(), "Partial file must remain intact");
}

#[test]
fn test_adversarial_atomic_replace_missing_source() {
    let dir = tempdir().expect("Failed to create tempdir");
    let non_existent_partial = dir.path().join("phantom.partial");
    let target = dir.path().join("target.dat");

    let res = PlatformAtomic::atomic_replace(&non_existent_partial, &target, false);
    assert!(
        matches!(res, Err(PlatformError::NotFound(_))),
        "Expected NotFound for missing source, got {:?}",
        res
    );
}

// ============================================================================
// AREA 2: ATOMIC SWAP (RENAME_SWAP)
// ============================================================================

#[test]
fn test_adversarial_atomic_swap_basic_and_unlink() {
    let dir = tempdir().expect("Failed to create tempdir");
    let target_dest = dir.path().join("final_file.dat");
    let source_partial = dir.path().join(".volterra.job1.final_file.dat.partial");

    fs::write(&target_dest, b"VERSION_1_DATA").expect("write target");
    fs::write(&source_partial, b"VERSION_2_UPGRADED_DATA").expect("write partial");

    let res = PlatformAtomic::atomic_replace(&source_partial, &target_dest, true);
    assert_eq!(
        res.unwrap(),
        AtomicReplaceResult::SwappedExisting,
        "Expected SwappedExisting"
    );

    // Target now holds Version 2
    assert_eq!(
        fs::read(&target_dest).expect("read target"),
        b"VERSION_2_UPGRADED_DATA"
    );

    // Source partial was cleanly removed after swap
    assert!(
        !source_partial.exists(),
        "Source partial should be unlinked after successful swap"
    );
}

#[test]
fn test_adversarial_atomic_swap_large_payload() {
    let dir = tempdir().expect("Failed to create tempdir");
    let target_dest = dir.path().join("large_target.bin");
    let source_partial = dir.path().join("large_partial.bin");

    let size = 4 * 1024 * 1024; // 4 MiB
    let payload_a = vec![0x33u8; size];
    let payload_b = vec![0x77u8; size];

    fs::write(&target_dest, &payload_a).expect("write a");
    fs::write(&source_partial, &payload_b).expect("write b");

    let res = PlatformAtomic::atomic_replace(&source_partial, &target_dest, true);
    assert!(res.is_ok(), "Large swap must succeed: {:?}", res);

    let read_back = fs::read(&target_dest).expect("read large target");
    assert_eq!(read_back.len(), size);
    assert_eq!(read_back, payload_b);
    assert!(!source_partial.exists());
}

#[test]
fn test_adversarial_atomic_swap_repeated_loop() {
    let dir = tempdir().expect("Failed to create tempdir");
    let target_dest = dir.path().join("loop_target.txt");
    fs::write(&target_dest, b"INITIAL").expect("write initial");

    for iteration in 0..25 {
        let partial_path = dir.path().join(format!("partial_{}.txt", iteration));
        let expected_text = format!("ITERATION_{}", iteration);
        fs::write(&partial_path, expected_text.as_bytes()).expect("write iteration");

        let res = PlatformAtomic::atomic_replace(&partial_path, &target_dest, true);
        assert_eq!(
            res.unwrap(),
            AtomicReplaceResult::SwappedExisting,
            "Swap failed at iteration {}",
            iteration
        );

        assert_eq!(
            fs::read_to_string(&target_dest).expect("read target"),
            expected_text,
            "Iteration {} data mismatch",
            iteration
        );
        assert!(!partial_path.exists());
    }
}

// ============================================================================
// AREA 3: DIRECT I/O & CACHE ADVISORY (fcntl F_NOCACHE, F_RDADVISE)
// ============================================================================

#[test]
fn test_adversarial_direct_io_fcntl_nocache_data_integrity() {
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("direct_io_stress.dat");

    // 1. Create file and enable direct I/O
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&file_path)
        .expect("open file");

    let fd = file.as_raw_fd();
    let res = PlatformIo::set_direct_io(fd);
    assert!(
        res.is_ok(),
        "set_direct_io must succeed on valid fd: {:?}",
        res
    );

    let res_global = PlatformIo::set_global_nocache(fd);
    assert!(
        res_global.is_ok(),
        "set_global_nocache must succeed: {:?}",
        res_global
    );

    // 2. Perform write operations while F_NOCACHE is active
    let mut writer = file;
    let payload = vec![0xEFu8; 128 * 1024]; // 128 KiB
    writer
        .write_all(&payload)
        .expect("write_all under direct I/O");
    writer.sync_all().expect("sync_all");

    // 3. Open second reader fd with direct I/O and verify data
    let reader_file = File::open(&file_path).expect("open reader");
    PlatformIo::set_direct_io(reader_file.as_raw_fd()).expect("set_direct_io reader");

    let mut buf = Vec::new();
    let mut r = reader_file;
    r.read_to_end(&mut buf)
        .expect("read_to_end under direct I/O");
    assert_eq!(buf.len(), payload.len());
    assert_eq!(buf, payload);
}

#[test]
fn test_adversarial_direct_io_invalid_fd_handling() {
    let bad_fds = [-1, -99, 999_999];
    for &bad_fd in &bad_fds {
        let res = PlatformIo::set_direct_io(bad_fd);
        assert!(
            matches!(
                res,
                Err(PlatformError::SyscallFailed {
                    op: "fcntl(F_NOCACHE)",
                    ..
                })
            ),
            "Expected SyscallFailed on bad fd {}, got {:?}",
            bad_fd,
            res
        );

        let res_global = PlatformIo::set_global_nocache(bad_fd);
        assert!(
            matches!(
                res_global,
                Err(PlatformError::SyscallFailed {
                    op: "fcntl(F_GLOBAL_NOCACHE)",
                    ..
                })
            ),
            "Expected SyscallFailed on bad fd {}, got {:?}",
            bad_fd,
            res_global
        );
    }
}

#[test]
fn test_adversarial_readahead_advisory_edge_cases() {
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("readahead_edge.dat");
    let file = File::create(&file_path).expect("create file");
    let fd = file.as_raw_fd();

    // Edge case 1: Length = 0 must short-circuit Ok(()) without syscall
    let res_zero = PlatformIo::advise_readahead(fd, 0, 0);
    assert!(res_zero.is_ok(), "Length 0 readahead must return Ok(())");

    // Edge case 2: Valid advisory
    let res_valid = PlatformIo::advise_readahead(fd, 0, 65536);
    assert!(
        res_valid.is_ok(),
        "Standard readahead advisory must succeed"
    );

    // Edge case 3: Offset far past EOF (e.g. 10 GiB on an empty file)
    let res_past_eof = PlatformIo::advise_readahead(fd, 10 * 1024 * 1024 * 1024, 1024 * 1024);
    assert!(
        res_past_eof.is_ok(),
        "Readahead past EOF should be handled gracefully by kernel"
    );

    // Edge case 4: Maximum clamped length (usize::MAX)
    let res_huge = PlatformIo::advise_readahead(fd, 0, usize::MAX);
    assert!(
        res_huge.is_ok(),
        "Massive length should be clamped safely without panic"
    );

    // Edge case 5: Sequential readahead advise (F_RDAHEAD)
    let res_seq = PlatformIo::advise_sequential(fd);
    assert!(
        res_seq.is_ok(),
        "advise_sequential must succeed on valid fd"
    );

    // Edge case 6: Bad fd must return error
    let res_bad = PlatformIo::advise_readahead(-1, 0, 4096);
    assert!(
        matches!(
            res_bad,
            Err(PlatformError::SyscallFailed {
                op: "fcntl(F_RDADVISE)",
                ..
            })
        ),
        "Expected error on bad fd, got {:?}",
        res_bad
    );

    let res_bad_seq = PlatformIo::advise_sequential(-1);
    assert!(
        matches!(
            res_bad_seq,
            Err(PlatformError::SyscallFailed {
                op: "fcntl(F_RDAHEAD)",
                ..
            })
        ),
        "Expected error on bad fd for sequential, got {:?}",
        res_bad_seq
    );
}

// ============================================================================
// AREA 4: DARWIN EXTENDED ATTRIBUTES (MULTI-ATTR & NON-ASCII)
// ============================================================================

#[test]
fn test_adversarial_xattr_multi_attribute_preservation() {
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("multi_xattr.dat");
    fs::write(&file_path, b"xattr multi test").expect("write");

    let num_attrs = 20;
    for i in 0..num_attrs {
        let name = format!("com.volterra.test.attr_{:03}", i);
        let val = format!("VALUE_{:03}_PAYLOAD", i).into_bytes();
        PlatformXattr::set_xattr(&file_path, &name, &val).expect("set_xattr");
    }

    // List attributes and verify all 20 are present
    let listed = PlatformXattr::list_xattrs(&file_path).expect("list_xattrs");
    for i in 0..num_attrs {
        let name = format!("com.volterra.test.attr_{:03}", i);
        assert!(
            listed.contains(&name),
            "Attribute '{}' missing from list_xattrs output: {:?}",
            name,
            listed
        );

        let retrieved = PlatformXattr::get_xattr(&file_path, &name).expect("get_xattr");
        let expected_val = format!("VALUE_{:03}_PAYLOAD", i).into_bytes();
        assert_eq!(retrieved, Some(expected_val));
    }
}

#[test]
fn test_adversarial_xattr_non_ascii_and_turkish_names() {
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("unicode_xattr.dat");
    fs::write(&file_path, b"unicode xattr test").expect("write");

    let unicode_cases = vec![
        ("com.volterra.öznitelik_şçğüıİ", b"TURKISH_PAYLOAD".to_vec()),
        ("com.volterra.rocket_🚀_tag", b"EMOJI_PAYLOAD".to_vec()),
        ("com.volterra.café_naïve_résumé", b"FRENCH_ACCENTS".to_vec()),
        ("com.volterra.日本語_データ", b"JAPANESE_PAYLOAD".to_vec()),
    ];

    for (name, val) in &unicode_cases {
        PlatformXattr::set_xattr(&file_path, name, val).expect("set_xattr unicode");
    }

    let listed = PlatformXattr::list_xattrs(&file_path).expect("list_xattrs");
    for (name, expected_val) in &unicode_cases {
        assert!(
            listed.contains(&name.to_string()),
            "Unicode attribute name '{}' missing from listed: {:?}",
            name,
            listed
        );

        let read_back = PlatformXattr::get_xattr(&file_path, name).expect("get_xattr");
        assert_eq!(read_back, Some(expected_val.clone()));
    }

    // Test removing one unicode xattr
    let to_remove = "com.volterra.rocket_🚀_tag";
    PlatformXattr::remove_xattr(&file_path, to_remove).expect("remove_xattr");
    let post_remove = PlatformXattr::get_xattr(&file_path, to_remove).expect("get after remove");
    assert_eq!(post_remove, None);
}

#[test]
fn test_adversarial_xattr_binary_and_empty_values() {
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("binary_xattr.dat");
    fs::write(&file_path, b"binary xattr").expect("write");

    // 1. Empty value (0 bytes)
    let empty_name = "com.volterra.empty";
    PlatformXattr::set_xattr(&file_path, empty_name, &[]).expect("set empty xattr");
    let empty_val = PlatformXattr::get_xattr(&file_path, empty_name).expect("get empty");
    assert_eq!(empty_val, Some(vec![]));

    // 2. Binary value (0x00 to 0xFF)
    let binary_name = "com.volterra.binary";
    let binary_payload: Vec<u8> = (0u8..=255).collect();
    PlatformXattr::set_xattr(&file_path, binary_name, &binary_payload).expect("set binary xattr");
    let binary_val = PlatformXattr::get_xattr(&file_path, binary_name).expect("get binary");
    assert_eq!(binary_val, Some(binary_payload));

    // 3. Moderately large value (16 KiB)
    let large_name = "com.volterra.large";
    let large_payload = vec![0xAAu8; 16 * 1024];
    PlatformXattr::set_xattr(&file_path, large_name, &large_payload).expect("set large xattr");
    let large_val = PlatformXattr::get_xattr(&file_path, large_name).expect("get large");
    assert_eq!(large_val, Some(large_payload));
}

#[test]
fn test_adversarial_metadata_copy_preserves_all_attributes() {
    let dir = tempdir().expect("Failed to create tempdir");
    let src = dir.path().join("source_meta.dat");
    let dst = dir.path().join("dest_meta.dat");

    fs::write(&src, b"SOURCE").expect("write src");
    fs::write(&dst, b"DEST").expect("write dst");

    // Populate src with diverse xattrs
    PlatformXattr::set_xattr(&src, "user.tag1", b"val1").expect("xattr 1");
    PlatformXattr::set_xattr(
        &src,
        "com.apple.metadata:kMDItemWhereFroms",
        b"https://volterra.app",
    )
    .expect("xattr 2");
    PlatformXattr::set_xattr(&src, "com.volterra.türkçe", b"veriler").expect("xattr 3");

    let preserved = PlatformMetadata::copy_metadata(&src, &dst).expect("copy_metadata");
    assert!(preserved.xattrs_count >= 3);
    assert_eq!(preserved.xattrs_dropped_count, 0);

    // Verify dst has all 3 xattrs
    let dst_val1 = PlatformXattr::get_xattr(&dst, "user.tag1").expect("get dst 1");
    assert_eq!(dst_val1, Some(b"val1".to_vec()));

    let dst_val3 = PlatformXattr::get_xattr(&dst, "com.volterra.türkçe").expect("get dst 3");
    assert_eq!(dst_val3, Some(b"veriler".to_vec()));
}

// ============================================================================
// AREA 5: NANOSECOND & EXTREME TIMESTAMPS
// ============================================================================

#[test]
fn test_adversarial_timestamp_nanosecond_precision() {
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("nano_time.dat");
    fs::write(&file_path, b"nanosecond fidelity").expect("write");

    let ts_target = FileTimestamps {
        btime_sec: 1_690_000_000,
        btime_nsec: 123_456_789,
        mtime_sec: 1_690_005_000,
        mtime_nsec: 987_654_321,
    };

    PlatformTime::set_timestamps(&file_path, &ts_target).expect("set_timestamps");
    let queried = PlatformTime::get_timestamps(&file_path).expect("get_timestamps");

    assert_eq!(queried.btime_sec, 1_690_000_000);
    assert_eq!(queried.mtime_sec, 1_690_005_000);

    // APFS supports nanosecond resolution
    assert_eq!(
        queried.btime_nsec, 123_456_789,
        "Birthtime nanoseconds did not preserve exact value"
    );
    assert_eq!(
        queried.mtime_nsec, 987_654_321,
        "Modification nanoseconds did not preserve exact value"
    );
}

#[test]
fn test_adversarial_timestamp_post_2038_extreme_future() {
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("future_time.dat");
    fs::write(&file_path, b"2038+ test").expect("write");

    // Year 2042: 2,272,060,800 (> i32::MAX = 2,147,483,647)
    let ts_2042 = FileTimestamps {
        btime_sec: 2_272_060_800,
        btime_nsec: 500_000_000,
        mtime_sec: 2_272_060_900,
        mtime_nsec: 750_000_000,
    };

    PlatformTime::set_timestamps(&file_path, &ts_2042).expect("set_timestamps for 2042");
    let queried = PlatformTime::get_timestamps(&file_path).expect("get_timestamps 2042");

    assert_eq!(
        queried.btime_sec, 2_272_060_800,
        "Post-2038 btime overflowed 32-bit boundary!"
    );
    assert_eq!(
        queried.mtime_sec, 2_272_060_900,
        "Post-2038 mtime overflowed 32-bit boundary!"
    );
    assert_eq!(queried.btime_nsec, 500_000_000);
    assert_eq!(queried.mtime_nsec, 750_000_000);

    // Year 2100: 4,102,444,800 (> u32::MAX / 2)
    let ts_2100 = FileTimestamps {
        btime_sec: 4_102_444_800,
        btime_nsec: 111_222_333,
        mtime_sec: 4_102_444_900,
        mtime_nsec: 444_555_666,
    };

    PlatformTime::set_timestamps(&file_path, &ts_2100).expect("set_timestamps for 2100");
    let queried_2100 = PlatformTime::get_timestamps(&file_path).expect("get_timestamps 2100");

    assert_eq!(queried_2100.btime_sec, 4_102_444_800);
    assert_eq!(queried_2100.mtime_sec, 4_102_444_900);
    assert_eq!(queried_2100.btime_nsec, 111_222_333);
    assert_eq!(queried_2100.mtime_nsec, 444_555_666);
}

#[test]
fn test_adversarial_timestamp_historical_and_zero_epoch() {
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("historical_time.dat");
    fs::write(&file_path, b"historical test").expect("write");

    // Year 1980 (e.g. MS-DOS epoch era): 315,532,800
    let ts_1980 = FileTimestamps {
        btime_sec: 315_532_800,
        btime_nsec: 100_000,
        mtime_sec: 315_532_900,
        mtime_nsec: 200_000,
    };

    PlatformTime::set_timestamps(&file_path, &ts_1980).expect("set 1980");
    let q_1980 = PlatformTime::get_timestamps(&file_path).expect("get 1980");
    assert_eq!(q_1980.btime_sec, 315_532_800);
    assert_eq!(q_1980.mtime_sec, 315_532_900);

    // Unix epoch 0 (1970-01-01 00:00:00 UTC)
    let ts_epoch0 = FileTimestamps {
        btime_sec: 0,
        btime_nsec: 0,
        mtime_sec: 0,
        mtime_nsec: 0,
    };
    PlatformTime::set_timestamps(&file_path, &ts_epoch0).expect("set epoch 0");
    let q_epoch0 = PlatformTime::get_timestamps(&file_path).expect("get epoch 0");
    assert_eq!(q_epoch0.btime_sec, 0);
    assert_eq!(q_epoch0.mtime_sec, 0);
}

#[test]
fn test_adversarial_copy_timestamps_fidelity() {
    let dir = tempdir().expect("Failed to create tempdir");
    let src = dir.path().join("time_src.dat");
    let dst = dir.path().join("time_dst.dat");

    fs::write(&src, b"SRC").expect("write");
    fs::write(&dst, b"DST").expect("write");

    let ts = FileTimestamps {
        btime_sec: 1_650_000_000,
        btime_nsec: 777_888_999,
        mtime_sec: 1_650_001_111,
        mtime_nsec: 111_222_333,
    };
    PlatformTime::set_timestamps(&src, &ts).expect("set src");
    PlatformTime::copy_timestamps(&src, &dst).expect("copy_timestamps");

    let dst_ts = PlatformTime::get_timestamps(&dst).expect("get dst");
    assert_eq!(dst_ts.btime_sec, ts.btime_sec);
    assert_eq!(dst_ts.btime_nsec, ts.btime_nsec);
    assert_eq!(dst_ts.mtime_sec, ts.mtime_sec);
    assert_eq!(dst_ts.mtime_nsec, ts.mtime_nsec);
}
