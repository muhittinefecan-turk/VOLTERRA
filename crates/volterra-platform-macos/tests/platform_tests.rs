use std::fs::File;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use volterra_platform_macos::{
    AtomicReplaceResult, FileTimestamps, FsType, MediaRotation, PlatformAtomic, PlatformError,
    PlatformFs, PlatformIo, PlatformMetadata, PlatformProfile, PlatformSymlink, PlatformTime,
    PlatformVolume, PlatformXattr,
};

#[test]
fn test_query_stats_tempdir() {
    let dir = tempdir().expect("Failed to create tempdir");
    let stats = PlatformFs::query_stats(dir.path()).expect("query_stats failed");

    assert_eq!(stats.block_size, 4096);
    assert!(stats.optimal_io_size > 0);
    assert!(stats.free_bytes > 0);
    assert_eq!(stats.fs_type, FsType::Apfs);
}

#[test]
fn test_query_stats_readonly_root() {
    let stats = PlatformFs::query_stats(Path::new("/")).expect("query_stats on / failed");
    assert!(
        stats.is_readonly,
        "macOS Sealed System Volume at / must be read-only"
    );
}

#[test]
fn test_get_volume_uuid_validity() {
    let dir = tempdir().expect("Failed to create tempdir");
    let uuid = PlatformVolume::get_volume_uuid(dir.path()).expect("get_volume_uuid failed");

    assert_ne!(
        uuid.0, [0u8; 16],
        "Volume UUID must be non-zero on active filesystem"
    );
    let formatted = uuid.to_string_formatted();
    assert_eq!(
        formatted.len(),
        36,
        "Formatted UUID must be 36 characters with hyphens"
    );
}

#[test]
fn test_media_rotation_ssd_or_unknown() {
    let dir = tempdir().expect("Failed to create tempdir");
    let rot = PlatformProfile::detect_media_rotation(dir.path())
        .expect("detect_media_rotation should not return Err");

    assert!(
        matches!(rot, MediaRotation::SolidState | MediaRotation::Unknown),
        "Internal NVMe or container should be SolidState or Unknown, got {:?}",
        rot
    );
}

#[test]
fn test_set_direct_io_flags() {
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("direct_io_test.bin");
    let file = File::create(&file_path).expect("Failed to create file");

    let res = PlatformIo::set_direct_io(file.as_raw_fd());
    assert!(res.is_ok(), "set_direct_io must succeed on valid fd");
}

#[test]
fn test_advise_readahead_valid() {
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("readahead_test.bin");
    let mut file = File::create(&file_path).expect("Failed to create file");

    let payload = vec![0xABu8; 1024 * 1024]; // 1 MiB
    file.write_all(&payload).expect("Failed to write payload");
    file.sync_all().expect("sync failed");

    let res = PlatformIo::advise_readahead(file.as_raw_fd(), 0, 65536);
    assert!(
        res.is_ok(),
        "advise_readahead must succeed on valid data file"
    );
}

#[test]
fn test_atomic_replace_new_file() {
    let dir = tempdir().expect("Failed to create tempdir");
    let partial_path = dir.path().join("foo.partial");
    let final_path = dir.path().join("foo.final");

    std::fs::write(&partial_path, b"Hello World").expect("Failed to write partial");
    assert!(!final_path.exists());

    let res = PlatformAtomic::atomic_replace(&partial_path, &final_path, false);
    assert_eq!(
        res.unwrap(),
        AtomicReplaceResult::PublishedNew,
        "atomic_replace must return PublishedNew for new destination"
    );
    assert!(final_path.exists(), "Final file must exist");
    assert!(!partial_path.exists(), "Partial file must not exist");
    assert_eq!(
        std::fs::read(&final_path).unwrap(),
        b"Hello World",
        "Final content must match"
    );
}

#[test]
fn test_atomic_replace_collision_blocked() {
    let dir = tempdir().expect("Failed to create tempdir");
    let final_path = dir.path().join("target.final");
    let partial_path = dir.path().join("target.partial");

    std::fs::write(&final_path, b"ORIGINAL DATA").expect("write final");
    std::fs::write(&partial_path, b"NEW DATA").expect("write partial");

    let res = PlatformAtomic::atomic_replace(&partial_path, &final_path, false);
    assert!(
        matches!(res, Err(PlatformError::TargetExists(_))),
        "atomic_replace must fail with TargetExists when overwrite disallowed"
    );
    assert_eq!(
        std::fs::read(&final_path).unwrap(),
        b"ORIGINAL DATA",
        "Original file must be untouched"
    );
    assert!(
        partial_path.exists(),
        "Partial file must remain intact for recovery"
    );
}

#[test]
fn test_atomic_replace_overwrite_swap() {
    let dir = tempdir().expect("Failed to create tempdir");
    let final_path = dir.path().join("target.final");
    let partial_path = dir.path().join("target.partial");

    std::fs::write(&final_path, b"OLD CONTENT").expect("write final");
    std::fs::write(&partial_path, b"NEW CONTENT").expect("write partial");

    let res = PlatformAtomic::atomic_replace(&partial_path, &final_path, true);
    assert!(
        res.is_ok(),
        "atomic_replace with overwrite approved must succeed"
    );
    assert_eq!(
        std::fs::read(&final_path).unwrap(),
        b"NEW CONTENT",
        "Target must contain new content"
    );
    assert!(
        !partial_path.exists(),
        "Old file unlinked safely after atomic swap"
    );
}

#[test]
fn test_xattr_roundtrip() {
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("xattr_file.bin");
    std::fs::write(&file_path, b"xattr test data").expect("write");

    let attr_name = "com.volterra.meta";
    let attr_val = b"12345";

    PlatformXattr::set_xattr(&file_path, attr_name, attr_val).expect("set_xattr failed");
    let read_back = PlatformXattr::get_xattr(&file_path, attr_name).expect("get_xattr failed");

    assert_eq!(read_back, Some(attr_val.to_vec()));

    // Also verify copy_metadata
    let dst_path = dir.path().join("xattr_dst.bin");
    std::fs::write(&dst_path, b"destination content").expect("write dst");
    let preserved =
        PlatformMetadata::copy_metadata(&file_path, &dst_path).expect("copy_metadata failed");
    assert!(preserved.xattrs_count >= 1);
}

#[test]
fn test_timestamp_preservation_btime_mtime() {
    let dir = tempdir().expect("Failed to create tempdir");
    let file_path = dir.path().join("time_file.bin");
    std::fs::write(&file_path, b"time test").expect("write");

    let target_ts = FileTimestamps {
        btime_sec: 1_600_000_000,
        btime_nsec: 0,
        mtime_sec: 1_700_000_000,
        mtime_nsec: 0,
    };

    PlatformTime::set_timestamps(&file_path, &target_ts).expect("set_timestamps failed");
    let queried = PlatformTime::get_timestamps(&file_path).expect("get_timestamps failed");

    assert_eq!(queried.btime_sec, 1_600_000_000);
    assert_eq!(queried.mtime_sec, 1_700_000_000);
}

#[test]
fn test_symlink_preserve_relative_and_dangling() {
    let dir = tempdir().expect("Failed to create tempdir");
    let dangling_src = dir.path().join("dangling.link");
    let dangling_dst = dir.path().join("dangling_copy.link");
    let non_existent_target = PathBuf::from("non_existent_target.txt");

    std::os::unix::fs::symlink(&non_existent_target, &dangling_src)
        .expect("create dangling symlink");

    assert!(PlatformSymlink::is_symlink(&dangling_src));
    PlatformSymlink::preserve_symlink(&dangling_src, &dangling_dst)
        .expect("preserve_symlink failed");

    assert!(
        PlatformSymlink::is_symlink(&dangling_dst),
        "Destination must be a symlink"
    );
    let target = PlatformSymlink::read_symlink(&dangling_dst).expect("read_symlink failed");
    assert_eq!(
        target, non_existent_target,
        "Preserved symlink target must match exactly"
    );
}
