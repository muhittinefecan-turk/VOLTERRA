//! Empirical stress test harness for `volterra-integrity`.
//!
//! Evaluates:
//! 1. BLAKE3 boundary conditions (0B, 1B, 8 MiB - 1, 8 MiB, 8 MiB + 1, 16 MiB).
//! 2. Streaming vs one-shot hasher equivalence across arbitrary chunk segmentations.
//! 3. Single-bit flip corruption detection across a 10 MiB buffer at boundary offsets.
//! 4. Truncation and short read fail-closed rejection.
//! 5. Direct I/O readback verification invariants.

use std::fs::{File, OpenOptions};
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use tempfile::tempdir;
use volterra_integrity::{
    Blake3FileHasher, Blake3Hasher, ChunkDigestAggregator, HashDigest, IntegrityError,
    IntegrityValidator, ReadbackVerifier, VerificationOutcome,
};

const CANONICAL_EMPTY_HEX: &str =
    "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

/// Generate a deterministic pseudo-random byte buffer of specified size.
fn generate_deterministic_buffer(size: usize, seed: u64) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(size);
    let mut state = seed;
    for _ in 0..size {
        // Linear congruential generator step
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        buffer.push((state >> 33) as u8);
    }
    buffer
}

#[test]
fn test_blake3_boundary_0_byte() {
    // 1. One-shot empty buffer
    let raw = Blake3Hasher::hash_chunk(&[]);
    let digest = HashDigest::new(raw);
    assert_eq!(digest.to_hex(), CANONICAL_EMPTY_HEX);

    // 2. Blake3FileHasher with zero updates
    let file_hasher = Blake3FileHasher::new();
    assert_eq!(file_hasher.bytes_hashed(), 0);
    assert_eq!(file_hasher.chunks_count(), 0);
    assert_eq!(file_hasher.finalize_digest().to_hex(), CANONICAL_EMPTY_HEX);

    // 3. Blake3FileHasher with empty slice update
    let mut file_hasher_empty_update = Blake3FileHasher::new();
    file_hasher_empty_update.update(&[]);
    assert_eq!(
        file_hasher_empty_update.finalize_digest().to_hex(),
        CANONICAL_EMPTY_HEX
    );

    // 4. Blake3Hasher::hash_slices with no slices / empty slices
    assert_eq!(
        HashDigest::new(Blake3Hasher::hash_slices(&[])).to_hex(),
        CANONICAL_EMPTY_HEX
    );
    assert_eq!(
        HashDigest::new(Blake3Hasher::hash_slices(&[&[], &[]])).to_hex(),
        CANONICAL_EMPTY_HEX
    );

    // 5. Blake3Hasher::hash_reader on empty cursor
    let mut empty_cursor = Cursor::new(&[]);
    let reader_digest = Blake3Hasher::hash_reader(&mut empty_cursor).expect("hash_reader");
    assert_eq!(reader_digest.to_hex(), CANONICAL_EMPTY_HEX);

    // 6. ReadbackVerifier on 0-byte file
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("empty_test.bin");
    File::create(&file_path).expect("create empty file");

    let verifier = ReadbackVerifier::new();
    let verified = verifier
        .verify_file(&file_path, &raw)
        .expect("verify 0-byte file");
    assert!(
        verified,
        "ReadbackVerifier must verify 0-byte file as matching"
    );

    // 7. IntegrityValidator on 0-byte file
    let file = File::open(&file_path).expect("open empty file");
    let pub_res = IntegrityValidator::validate_file_for_publication(
        &file,
        Some(&file_path),
        0,
        Some(&raw),
        &verifier,
    );
    assert!(
        pub_res.is_ok(),
        "IntegrityValidator must accept valid 0-byte file"
    );
}

#[test]
fn test_blake3_boundary_1_byte() {
    let test_bytes: &[u8] = &[0x00, 0x01, 0x42, 0x7F, 0x80, 0xFE, 0xFF];

    for &b in test_bytes {
        let single_slice = [b];

        // One-shot
        let expected_raw = Blake3Hasher::hash_chunk(&single_slice);
        let expected_digest = HashDigest::new(expected_raw);

        // Streaming file hasher
        let mut file_hasher = Blake3FileHasher::new();
        file_hasher.update(&single_slice);
        assert_eq!(file_hasher.bytes_hashed(), 1);
        assert_eq!(file_hasher.finalize(), expected_raw);

        // Reader streaming
        let mut cursor = Cursor::new(&single_slice);
        let reader_digest = Blake3Hasher::hash_reader(&mut cursor).expect("hash_reader");
        assert_eq!(reader_digest, expected_digest);

        // Slices
        let slices_raw = Blake3Hasher::hash_slices(&[&single_slice]);
        assert_eq!(slices_raw, expected_raw);

        // Readback verification on disk
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join(format!("byte_{:02x}.bin", b));
        fs_write_sync(&file_path, &single_slice);

        let verifier = ReadbackVerifier::new();
        let verified = verifier
            .verify_file(&file_path, &expected_raw)
            .expect("verify 1-byte file");
        assert!(verified, "ReadbackVerifier must match 1-byte file");
    }
}

#[test]
fn test_blake3_boundary_8mib_boundaries() {
    const MIB_8: usize = 8 * 1024 * 1024;
    let test_sizes = [
        MIB_8 - 1, // 8,388,607 bytes
        MIB_8,     // 8,388,608 bytes (exact chunk boundary)
        MIB_8 + 1, // 8,388,609 bytes
    ];

    for &size in &test_sizes {
        let buffer = generate_deterministic_buffer(size, 0xDEADBEEF + size as u64);

        // Canonical one-shot digest
        let expected_digest = Blake3Hasher::hash_chunk(&buffer);

        // 1. Streaming via small chunks (64 KiB)
        let mut hasher_64k = Blake3FileHasher::new();
        for chunk in buffer.chunks(64 * 1024) {
            hasher_64k.update(chunk);
        }
        assert_eq!(hasher_64k.bytes_hashed(), size as u64);
        assert_eq!(
            hasher_64k.finalize(),
            expected_digest,
            "64 KiB streaming must match one-shot for size {}",
            size
        );

        // 2. Streaming via 1 MiB chunks
        let mut hasher_1m = Blake3FileHasher::new();
        for chunk in buffer.chunks(1024 * 1024) {
            hasher_1m.update(chunk);
        }
        assert_eq!(
            hasher_1m.finalize(),
            expected_digest,
            "1 MiB streaming must match one-shot for size {}",
            size
        );

        // 3. Streaming via prime/odd chunk sizes (13,107 bytes)
        let mut hasher_odd = Blake3FileHasher::new();
        for chunk in buffer.chunks(13107) {
            hasher_odd.update(chunk);
        }
        assert_eq!(
            hasher_odd.finalize(),
            expected_digest,
            "Odd-size streaming must match one-shot for size {}",
            size
        );

        // 4. hash_reader verification
        let mut cursor = Cursor::new(&buffer);
        let reader_digest = Blake3Hasher::hash_reader(&mut cursor).expect("hash_reader");
        assert_eq!(
            reader_digest.as_bytes(),
            &expected_digest,
            "hash_reader must match one-shot for size {}",
            size
        );
    }
}

#[test]
fn test_single_bit_flip_avalanche_and_readback_10mib() {
    const SIZE: usize = 10 * 1024 * 1024; // 10 MiB
    let mut buffer = generate_deterministic_buffer(SIZE, 0xCAFEBABEDEADBEEF);
    let original_raw = Blake3Hasher::hash_chunk(&buffer);
    let original_digest = HashDigest::new(original_raw);

    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("ten_mib.bin");
    fs_write_sync(&file_path, &buffer);

    let verifier = ReadbackVerifier::new();

    // Baseline: unmutated file must pass
    let genuine_match = verifier
        .verify_file(&file_path, &original_raw)
        .expect("verify original");
    assert!(genuine_match, "Original file must match");

    // Critical bit flip offsets to test:
    // 1. Byte 0, bit 0 (very first bit of file)
    // 2. Byte 1024, bit 4 (within first chunk)
    // 3. Byte 8,388,607, bit 7 (last bit of first 8 MiB chunk)
    // 4. Byte 8,388,608, bit 0 (first bit of second chunk)
    // 5. Byte 10,485,759, bit 7 (very last bit of 10 MiB file)
    let flip_targets = [
        (0usize, 0x01u8),
        (1024usize, 0x10u8),
        (8 * 1024 * 1024 - 1, 0x80u8),
        (8 * 1024 * 1024, 0x01u8),
        (SIZE - 1, 0x80u8),
    ];

    for &(offset, bitmask) in &flip_targets {
        // Mutate bit in memory
        buffer[offset] ^= bitmask;
        let mutated_raw = Blake3Hasher::hash_chunk(&buffer);
        assert_ne!(
            mutated_raw, original_raw,
            "BLAKE3 avalanche failure: flipped bit at offset {} produced identical hash!",
            offset
        );

        // Mutate single byte on disk
        let mut file = OpenOptions::new()
            .write(true)
            .open(&file_path)
            .expect("open file for bit flip");
        file.seek(SeekFrom::Start(offset as u64)).expect("seek");
        file.write_all(&[buffer[offset]])
            .expect("write mutated byte");
        file.sync_all().expect("sync");
        drop(file);

        // 1. ReadbackVerifier::verify_file must return Ok(false)
        let verified = verifier
            .verify_file(&file_path, &original_raw)
            .expect("verify_file");
        assert!(
            !verified,
            "ReadbackVerifier failed to detect single bit flip at offset {}",
            offset
        );

        // 2. ReadbackVerifier::verify_range_outcome must return VerificationOutcome::Mismatch
        let file_ro = File::open(&file_path).expect("open ro");
        let outcome = verifier
            .verify_range_outcome(
                file_ro.as_raw_fd(),
                0,
                &original_digest,
                std::cmp::min(SIZE, 8 * 1024 * 1024),
            )
            .expect("verify_range_outcome");

        if offset < 8 * 1024 * 1024 {
            assert!(
                matches!(outcome, VerificationOutcome::Mismatch { .. }),
                "verify_range_outcome must detect mismatch in first chunk"
            );
        }

        // 3. IntegrityValidator must reject publication with ChecksumMismatch
        let val_res = IntegrityValidator::validate_file_for_publication(
            &file_ro,
            Some(&file_path),
            SIZE as u64,
            Some(&original_raw),
            &verifier,
        );
        match val_res {
            Err(IntegrityError::ChecksumMismatch { .. }) => {
                // Expected fail-closed rejection
            }
            other => panic!(
                "Expected ChecksumMismatch on bit flip at offset {}, got {:?}",
                offset, other
            ),
        }

        // Restore bit on disk & memory for clean baseline in next iteration
        buffer[offset] ^= bitmask;
        let mut restore_file = OpenOptions::new()
            .write(true)
            .open(&file_path)
            .expect("open file for restore");
        restore_file
            .seek(SeekFrom::Start(offset as u64))
            .expect("seek");
        restore_file
            .write_all(&[buffer[offset]])
            .expect("write restore");
        restore_file.sync_all().expect("sync");
        drop(restore_file);
    }
}

#[test]
fn test_truncated_buffers_fail_closed_rejection() {
    const EXPECTED_SIZE: usize = 8 * 1024 * 1024; // 8 MiB
    let full_buffer = generate_deterministic_buffer(EXPECTED_SIZE, 0x1122334455667788);
    let expected_raw = Blake3Hasher::hash_chunk(&full_buffer);

    let dir = tempdir().expect("tempdir");

    // Case 1: Truncated by exactly 1 byte (8 MiB - 1 on disk)
    let trunc_1b_path = dir.path().join("trunc_1b.bin");
    fs_write_sync(&trunc_1b_path, &full_buffer[..EXPECTED_SIZE - 1]);

    let verifier = ReadbackVerifier::new();
    let file_1b = File::open(&trunc_1b_path).expect("open trunc 1b");

    // IntegrityValidator must reject publication as TruncatedFile
    let val_res = IntegrityValidator::validate_file_for_publication(
        &file_1b,
        Some(&trunc_1b_path),
        EXPECTED_SIZE as u64,
        Some(&expected_raw),
        &verifier,
    );
    match val_res {
        Err(IntegrityError::TruncatedFile {
            expected_bytes,
            actual_bytes,
            ..
        }) => {
            assert_eq!(expected_bytes, EXPECTED_SIZE as u64);
            assert_eq!(actual_bytes, (EXPECTED_SIZE - 1) as u64);
        }
        other => panic!("Expected TruncatedFile error, got {:?}", other),
    }

    // verify_range with len = 8 MiB on truncated file must return UnexpectedEof
    let range_res = verifier.verify_range(file_1b.as_raw_fd(), 0, &expected_raw, EXPECTED_SIZE);
    match range_res {
        Err(IntegrityError::UnexpectedEof {
            expected_bytes,
            actual_bytes,
            ..
        }) => {
            assert_eq!(expected_bytes, EXPECTED_SIZE as u64);
            assert_eq!(actual_bytes, (EXPECTED_SIZE - 1) as u64);
        }
        other => panic!(
            "Expected UnexpectedEof on truncated readback, got {:?}",
            other
        ),
    }

    // Case 2: Zero-byte truncated file (expected 8 MiB, found 0)
    let empty_path = dir.path().join("empty_trunc.bin");
    File::create(&empty_path).expect("create empty");
    let empty_file = File::open(&empty_path).expect("open empty");

    let val_empty = IntegrityValidator::validate_file_for_publication(
        &empty_file,
        Some(&empty_path),
        EXPECTED_SIZE as u64,
        Some(&expected_raw),
        &verifier,
    );
    assert!(matches!(
        val_empty,
        Err(IntegrityError::TruncatedFile {
            actual_bytes: 0,
            ..
        })
    ));

    // Case 3: Oversized file (expected 8 MiB, actual 8 MiB + 1)
    let oversized_path = dir.path().join("oversized.bin");
    let mut over_buffer = full_buffer.clone();
    over_buffer.push(0xFF);
    fs_write_sync(&oversized_path, &over_buffer);

    let over_file = File::open(&oversized_path).expect("open oversized");
    let val_over = IntegrityValidator::validate_file_for_publication(
        &over_file,
        Some(&oversized_path),
        EXPECTED_SIZE as u64,
        Some(&expected_raw),
        &verifier,
    );
    assert!(matches!(
        val_over,
        Err(IntegrityError::OversizedFile { .. })
    ));

    // Case 4: Incomplete chunk set validation
    let incomplete_res = IntegrityValidator::validate_chunks_complete(10, 9);
    assert!(matches!(
        incomplete_res,
        Err(IntegrityError::IncompleteChunkSet {
            total_chunks: 10,
            verified_chunks: 9,
        })
    ));
}

#[test]
fn test_digest_hex_validation_and_aggregation_avalanche() {
    // 1. Valid hex roundtrip
    let raw = Blake3Hasher::hash_chunk(b"VOLTERRA cryptographic validation");
    let digest = HashDigest::new(raw);
    let hex_str = digest.to_hex();
    assert_eq!(hex_str.len(), 64);
    let parsed = HashDigest::from_hex(&hex_str).expect("from_hex valid");
    assert_eq!(parsed, digest);

    // 2. Invalid length (< 64, > 64)
    assert!(HashDigest::from_hex("abcd").is_err());
    assert!(HashDigest::from_hex(&format!("{}00", hex_str)).is_err());

    // 3. Non-hex characters
    let mut bad_hex = hex_str.clone();
    bad_hex.replace_range(0..2, "ZZ");
    assert!(HashDigest::from_hex(&bad_hex).is_err());

    // 4. Aggregation avalanche: changing one chunk digest changes aggregate completely
    let d1 = HashDigest::new(Blake3Hasher::hash_chunk(b"chunk1"));
    let d2 = HashDigest::new(Blake3Hasher::hash_chunk(b"chunk2"));
    let d3 = HashDigest::new(Blake3Hasher::hash_chunk(b"chunk3"));

    let agg_original = ChunkDigestAggregator::aggregate(&[d1, d2, d3]);

    // Flip bit in d2
    let mut d2_bytes = *d2.as_bytes();
    d2_bytes[15] ^= 0x01;
    let d2_mutated = HashDigest::new(d2_bytes);

    let agg_mutated = ChunkDigestAggregator::aggregate(&[d1, d2_mutated, d3]);
    assert_ne!(
        agg_original, agg_mutated,
        "ChunkDigestAggregator must produce distinct aggregate when any chunk changes"
    );
}

fn fs_write_sync(path: &std::path::Path, data: &[u8]) {
    let mut f = File::create(path).expect("create file");
    f.write_all(data).expect("write data");
    f.sync_all().expect("sync_all");
}
