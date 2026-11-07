use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use tempfile::tempdir;
use volterra_integrity::{
    Blake3FileHasher, Blake3Hasher, ChunkDigestAggregator, HashDigest, IntegrityValidator,
    ReadbackVerifier,
};

#[test]
fn test_blake3_known_vector_empty() {
    let empty_hash = Blake3Hasher::hash_chunk(&[]);
    let digest = HashDigest::new(empty_hash);
    assert_eq!(
        digest.to_hex(),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        "Empty buffer must match canonical BLAKE3 empty hash"
    );
}

#[test]
fn test_blake3_known_vector_pattern() {
    let input = b"The quick brown fox jumps over the lazy dog";
    let hash = Blake3Hasher::hash_chunk(input);
    let digest = HashDigest::new(hash);
    assert_eq!(
        digest.to_hex(),
        "2f1514181aadccd913abd94cfa592701a5686ab23f8df1dff1b74710febc6d4a",
        "Must match canonical BLAKE3 test vector for standard test sentence"
    );
}

#[test]
fn test_incremental_matches_single_pass() {
    // 16 MiB pseudo-random pattern
    let total_size = 16 * 1024 * 1024;
    let mut data = Vec::with_capacity(total_size);
    for i in 0..total_size {
        data.push(((i * 31 + 17) & 0xFF) as u8);
    }

    let single_pass = Blake3Hasher::hash_chunk(&data);

    // Stream through Blake3FileHasher in 8x 2 MiB chunks
    let mut hasher_2mb = Blake3FileHasher::new();
    let chunk_size = 2 * 1024 * 1024;
    for chunk in data.chunks(chunk_size) {
        hasher_2mb.update(chunk);
    }
    assert_eq!(hasher_2mb.bytes_hashed(), total_size as u64);
    assert_eq!(hasher_2mb.finalize(), single_pass);

    // Stream through Blake3FileHasher in 16x 1 MiB chunks
    let mut hasher_1mb = Blake3FileHasher::new();
    let chunk_size_1mb = 1024 * 1024;
    for chunk in data.chunks(chunk_size_1mb) {
        hasher_1mb.update(chunk);
    }
    assert_eq!(hasher_1mb.finalize(), single_pass);
}

#[test]
fn test_corruption_detection_single_bit() {
    let size = 1024 * 1024; // 1 MiB
    let mut buffer = vec![0x55u8; size];
    let original_hash = Blake3Hasher::hash_chunk(&buffer);

    // Flip 1 single bit at byte offset 500,000
    buffer[500_000] ^= 0x01;
    let corrupted_hash = Blake3Hasher::hash_chunk(&buffer);

    assert_ne!(
        original_hash, corrupted_hash,
        "Single bit flip must result in completely different hash (avalanche effect)"
    );

    // Verify chunk digest aggregation
    let digest1 = HashDigest::new(original_hash);
    let digest2 = HashDigest::new(corrupted_hash);
    let agg1 = ChunkDigestAggregator::aggregate(&[digest1]);
    let agg2 = ChunkDigestAggregator::aggregate(&[digest2]);
    assert_ne!(agg1, agg2);
}

#[test]
fn test_readback_verify_genuine_file() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("genuine.bin");

    let payload = vec![0x7Eu8; 4 * 1024 * 1024]; // 4 MiB
    let expected_hash = Blake3Hasher::hash_chunk(&payload);

    let mut file = File::create(&file_path).expect("create file");
    file.write_all(&payload).expect("write");
    file.sync_all().expect("sync");

    let verifier = ReadbackVerifier::new();
    let result = verifier
        .verify_file(&file_path, &expected_hash)
        .expect("verify_file");
    assert!(result, "Genuine uncorrupted file must verify as true");

    // Also test through IntegrityValidator
    let file_ro = File::open(&file_path).expect("open ro");
    let val_res = IntegrityValidator::validate_file_for_publication(
        &file_ro,
        Some(&file_path),
        payload.len() as u64,
        Some(&expected_hash),
        &verifier,
    );
    assert!(val_res.is_ok());
}

#[test]
fn test_readback_verify_corrupted_byte() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("corrupted.bin");

    let payload = vec![0x33u8; 4 * 1024 * 1024]; // 4 MiB
    let expected_hash = Blake3Hasher::hash_chunk(&payload);

    let mut file = File::create(&file_path).expect("create");
    file.write_all(&payload).expect("write");
    file.sync_all().expect("sync");

    // Mutate byte at offset 2,000,000 directly on disk
    let mut mutate_file = OpenOptions::new()
        .write(true)
        .open(&file_path)
        .expect("open for write");
    mutate_file.seek(SeekFrom::Start(2_000_000)).expect("seek");
    mutate_file.write_all(&[0xFF]).expect("write mutated byte");
    mutate_file.sync_all().expect("sync");

    let verifier = ReadbackVerifier::new();
    let result = verifier
        .verify_file(&file_path, &expected_hash)
        .expect("verify_file");
    assert!(
        !result,
        "Corrupted file on disk must fail verification and return false"
    );

    // Confirm fail-closed publication rejection
    let read_file = File::open(&file_path).expect("open");
    let val_res = IntegrityValidator::validate_file_for_publication(
        &read_file,
        Some(&file_path),
        payload.len() as u64,
        Some(&expected_hash),
        &verifier,
    );
    assert!(
        val_res.is_err(),
        "IntegrityValidator must abort publication of corrupted file"
    );
}

#[test]
fn test_readback_range_verification() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("range.bin");

    let total_size = 4 * 1024 * 1024;
    let mut data = Vec::with_capacity(total_size);
    for i in 0..total_size {
        data.push((i % 251) as u8);
    }

    let mut file = File::create(&file_path).expect("create");
    file.write_all(&data).expect("write");
    file.sync_all().expect("sync");

    // Sub-range [offset: 1048576, len: 1048576] (chunk 1)
    let offset = 1024 * 1024;
    let len = 1024 * 1024;
    let expected_sub_hash = Blake3Hasher::hash_chunk(&data[offset..offset + len]);

    let verifier = ReadbackVerifier::new();
    let file_ro = File::open(&file_path).expect("open ro");
    let matches = verifier
        .verify_range(file_ro.as_raw_fd(), offset as u64, &expected_sub_hash, len)
        .expect("verify_range");
    assert!(
        matches,
        "Sub-range readback verification must match expected chunk digest"
    );
}

#[test]
fn test_readback_nocache_flag_enforced() {
    let verifier = ReadbackVerifier::new();
    assert!(
        verifier.is_direct_io_enabled(),
        "ReadbackVerifier must enable direct I/O by default to bypass OS cache"
    );

    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("nocache_probe.bin");
    let file = File::create(&file_path).expect("create");

    // Verify fcntl(fd, F_NOCACHE, 1) succeeds on file descriptor
    let ret = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_NOCACHE, 1) };
    assert_eq!(ret, 0, "fcntl F_NOCACHE must return 0 on active descriptor");
}
