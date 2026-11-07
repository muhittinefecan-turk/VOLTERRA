use crate::error::IntegrityError;
use blake3::Hasher;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Read;

pub const DEFAULT_CHUNK_SIZE: usize = 8 * 1024 * 1024; // 8 MiB
pub const RECOVERY_CHUNK_SIZE: usize = 1024 * 1024; // 1 MiB

/// 256-bit BLAKE3 digest representation.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HashDigest(pub [u8; 32]);

impl HashDigest {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for byte in &self.0 {
            use std::fmt::Write;
            let _ = write!(s, "{:02x}", byte);
        }
        s
    }

    pub fn from_hex(hex_str: &str) -> Result<Self, IntegrityError> {
        if hex_str.len() != 64 {
            return Err(IntegrityError::InvalidDigestFormat(format!(
                "Digest must be 64 hex characters, got {}",
                hex_str.len()
            )));
        }
        let mut bytes = [0u8; 32];
        for i in 0..32 {
            bytes[i] = u8::from_str_radix(&hex_str[i * 2..i * 2 + 2], 16)
                .map_err(|_| IntegrityError::InvalidDigestFormat(hex_str.to_string()))?;
        }
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl From<[u8; 32]> for HashDigest {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl From<HashDigest> for [u8; 32] {
    fn from(digest: HashDigest) -> Self {
        digest.0
    }
}

impl fmt::Debug for HashDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HashDigest({})", self.to_hex())
    }
}

impl fmt::Display for HashDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// One-shot and streaming BLAKE3 hashing utilities.
pub struct Blake3Hasher;

impl Blake3Hasher {
    /// Computes 256-bit raw BLAKE3 digest for an arbitrary byte buffer.
    pub fn hash_chunk(buffer: &[u8]) -> [u8; 32] {
        let hash = blake3::hash(buffer);
        *hash.as_bytes()
    }

    /// Computes BLAKE3 digest across multiple byte slices.
    pub fn hash_slices(slices: &[&[u8]]) -> [u8; 32] {
        let mut hasher = Hasher::new();
        for slice in slices {
            hasher.update(slice);
        }
        *hasher.finalize().as_bytes()
    }

    /// Computes HashDigest for an arbitrary slice.
    pub fn hash_to_digest(buffer: &[u8]) -> HashDigest {
        HashDigest(Self::hash_chunk(buffer))
    }

    /// Computes BLAKE3 digest by streaming an arbitrary reader in bounded buffers.
    pub fn hash_reader<R: Read>(reader: &mut R) -> Result<HashDigest, std::io::Error> {
        let mut hasher = Hasher::new();
        let mut buffer = [0u8; 64 * 1024]; // 64 KiB transfer buffer
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    hasher.update(&buffer[..n]);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        let output = hasher.finalize();
        Ok(HashDigest(*output.as_bytes()))
    }
}

/// Incremental whole-file hasher that ingests chunks in sequential order.
pub struct Blake3FileHasher {
    hasher: Hasher,
    bytes_hashed: u64,
    chunks_count: usize,
}

impl Default for Blake3FileHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Blake3FileHasher {
    pub fn new() -> Self {
        Self {
            hasher: Hasher::new(),
            bytes_hashed: 0,
            chunks_count: 0,
        }
    }

    /// Appends sequential raw chunk bytes into the whole-file BLAKE3 tree.
    pub fn update(&mut self, chunk_bytes: &[u8]) {
        self.hasher.update(chunk_bytes);
        self.bytes_hashed += chunk_bytes.len() as u64;
        self.chunks_count += 1;
    }

    /// Returns the number of bytes ingested so far.
    pub fn bytes_hashed(&self) -> u64 {
        self.bytes_hashed
    }

    /// Returns the number of chunks ingested so far.
    pub fn chunks_count(&self) -> usize {
        self.chunks_count
    }

    /// Finalizes and produces the canonical whole-file 256-bit raw digest.
    pub fn finalize(self) -> [u8; 32] {
        let output = self.hasher.finalize();
        *output.as_bytes()
    }

    /// Finalizes and produces the strongly-typed HashDigest.
    pub fn finalize_digest(self) -> HashDigest {
        HashDigest(self.finalize())
    }
}

/// Deterministically combines a sequence of chunk digests into a composite digest.
pub struct ChunkDigestAggregator;

impl ChunkDigestAggregator {
    pub fn aggregate(chunk_digests: &[HashDigest]) -> HashDigest {
        let mut hasher = Hasher::new_keyed(&[
            0x56, 0x4f, 0x4c, 0x54, 0x45, 0x52, 0x52, 0x41, // "VOLTERRA"
            0x5f, 0x43, 0x48, 0x55, 0x4e, 0x4b, 0x5f, 0x41, 0x47, 0x47, 0x52, 0x45, 0x47, 0x41,
            0x54, 0x45, 0x5f, 0x56, 0x31, 0x2e, 0x30, 0x2e, 0x30, 0x00,
        ]);
        for digest in chunk_digests {
            hasher.update(digest.as_bytes());
        }
        HashDigest(*hasher.finalize().as_bytes())
    }

    pub fn aggregate_raw(chunk_digests: &[[u8; 32]]) -> [u8; 32] {
        let mut hasher = Hasher::new_keyed(&[
            0x56, 0x4f, 0x4c, 0x54, 0x45, 0x52, 0x52, 0x41, 0x5f, 0x43, 0x48, 0x55, 0x4e, 0x4b,
            0x5f, 0x41, 0x47, 0x47, 0x52, 0x45, 0x47, 0x41, 0x54, 0x45, 0x5f, 0x56, 0x31, 0x2e,
            0x30, 0x2e, 0x30, 0x00,
        ]);
        for digest in chunk_digests {
            hasher.update(digest);
        }
        *hasher.finalize().as_bytes()
    }
}
