//! Reusable Synthetic Dataset Generator and Validator Fixtures
//! Designed for VOLTERRA Final Milestone (TEST-07 to TEST-10)

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use volterra_integrity::{Blake3FileHasher, Blake3Hasher, HashDigest};

pub const DARWIN_SEEK_HOLE: libc::c_int = 3;
pub const DARWIN_SEEK_DATA: libc::c_int = 4;

/// Metadata expectation for an expected synthetic file
#[derive(Debug, Clone)]
pub struct FileExpectation {
    pub relative_path: PathBuf,
    pub size_bytes: u64,
    pub blake3_digest: HashDigest,
    pub is_sparse: bool,
    pub max_allocated_blocks: Option<u64>,
}

/// Fluent builder for creating reproducible synthetic datasets on physical disk.
pub struct SyntheticDatasetBuilder {
    root: PathBuf,
    expectations: HashMap<PathBuf, FileExpectation>,
}

impl SyntheticDatasetBuilder {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            expectations: HashMap::new(),
        }
    }

    /// Root directory path
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Access collected expectations
    pub fn expectations(&self) -> &HashMap<PathBuf, FileExpectation> {
        &self.expectations
    }

    // ========================================================================
    // TEST-07: Boundary Edge Cases (0B, 1B, 8 MiB ± 1, 64 MiB ± 1)
    // ========================================================================
    pub fn add_boundary_files(&mut self) -> Result<&mut Self, std::io::Error> {
        let boundaries = [
            ("empty_0b.bin", 0u64),
            ("single_1b.bin", 1u64),
            ("boundary_8mib_minus_1.bin", 8 * 1024 * 1024 - 1),
            ("boundary_8mib_exact.bin", 8 * 1024 * 1024),
            ("boundary_8mib_plus_1.bin", 8 * 1024 * 1024 + 1),
            ("boundary_64mib_minus_1.bin", 64 * 1024 * 1024 - 1),
            ("boundary_64mib_exact.bin", 64 * 1024 * 1024),
            ("boundary_64mib_plus_1.bin", 64 * 1024 * 1024 + 1),
        ];

        let base_dir = self.root.join("boundary_tests");
        fs::create_dir_all(&base_dir)?;

        for (filename, size) in boundaries {
            let file_path = base_dir.join(filename);
            let rel_path = PathBuf::from("boundary_tests").join(filename);
            let mut file = File::create(&file_path)?;

            let digest = if size == 0 {
                Blake3Hasher::hash_to_digest(b"")
            } else {
                let mut hasher = Blake3FileHasher::new();
                let pattern_chunk_size = 64 * 1024; // 64 KiB buffer
                let mut remaining = size;
                let mut counter = 0u32;

                while remaining > 0 {
                    let to_write = std::cmp::min(remaining, pattern_chunk_size as u64) as usize;
                    let mut buf = vec![0u8; to_write];
                    for (i, b) in buf.iter_mut().enumerate() {
                        *b = ((counter + i as u32) & 0xFF) as u8;
                    }
                    file.write_all(&buf)?;
                    hasher.update(&buf);
                    remaining -= to_write as u64;
                    counter = counter.wrapping_add(to_write as u32);
                }
                file.sync_all()?;
                hasher.finalize_digest()
            };

            self.expectations.insert(
                rel_path.clone(),
                FileExpectation {
                    relative_path: rel_path,
                    size_bytes: size,
                    blake3_digest: digest,
                    is_sparse: false,
                    max_allocated_blocks: None,
                },
            );
        }

        Ok(self)
    }

    // ========================================================================
    // TEST-08: 10,000 Small Files Workload
    // ========================================================================
    pub fn add_small_files_workload(
        &mut self,
        total_files: usize,
        dir_fanout: usize,
    ) -> Result<&mut Self, std::io::Error> {
        let base_dir = self.root.join("small_files_10k");
        fs::create_dir_all(&base_dir)?;

        let files_per_dir = total_files.div_ceil(dir_fanout);
        let mut file_idx = 0;

        for d in 0..dir_fanout {
            let dir_name = format!("dir_{:03}", d);
            let dir_path = base_dir.join(&dir_name);
            fs::create_dir_all(&dir_path)?;

            for _ in 0..files_per_dir {
                if file_idx >= total_files {
                    break;
                }

                let filename = format!("small_{:05}.dat", file_idx);
                let file_path = dir_path.join(&filename);
                let rel_path = PathBuf::from("small_files_10k")
                    .join(&dir_name)
                    .join(&filename);

                // Varied small sizes: 16B, 64B, 256B, 1KB, 4KB
                let size = match file_idx % 5 {
                    0 => 16,
                    1 => 64,
                    2 => 256,
                    3 => 1024,
                    _ => 4096,
                };

                let mut payload = Vec::with_capacity(size);
                for i in 0..size {
                    payload.push(((file_idx + i) & 0xFF) as u8);
                }

                fs::write(&file_path, &payload)?;
                let digest = Blake3Hasher::hash_to_digest(&payload);

                self.expectations.insert(
                    rel_path.clone(),
                    FileExpectation {
                        relative_path: rel_path,
                        size_bytes: size as u64,
                        blake3_digest: digest,
                        is_sparse: false,
                        max_allocated_blocks: None,
                    },
                );

                file_idx += 1;
            }
        }

        Ok(self)
    }

    // ========================================================================
    // TEST-09: 25-Level Deep Unicode & Turkish Hierarchy
    // ========================================================================
    pub fn add_deep_unicode_hierarchy(
        &mut self,
        depth: usize,
    ) -> Result<&mut Self, std::io::Error> {
        let components = [
            "VOLTERRA_Test_Türkçe_ĞÜŞİÖç_Dosyalar", // Root level
            "Düzey_02_İstanbul_Kadıköy",            // Level 2
            "Düzey_03_çıkış_şebeke_öğrenci",        // Level 3
            "Düzey_04_📁_⚡️_🚀_🎉",                 // Level 4 (Emojis)
            "Düzey_05_Spaces & (Punctuation) [v1]", // Level 5 (Spaces & Punctuation)
            "Düzey_06_café_résumé_münchen",         // Level 6 (Accents)
            "Düzey_07_Dotted_İ_Dotless_ı",          // Level 7 (Turkish I variants)
        ];

        let mut cur_dir = self.root.clone();
        let mut rel_dir = PathBuf::new();

        // Build first 7 descriptive levels
        for comp in components {
            cur_dir = cur_dir.join(comp);
            rel_dir = rel_dir.join(comp);
            fs::create_dir_all(&cur_dir)?;
        }

        // Build remaining levels up to depth (compact Turkish names to respect PATH_MAX 1024)
        for d in 8..=depth {
            let comp = format!("lvl_{:02}_tr_ğüşöç", d);
            cur_dir = cur_dir.join(&comp);
            rel_dir = rel_dir.join(&comp);
            fs::create_dir_all(&cur_dir)?;
        }

        // Terminal payload file at 25th level
        let terminal_file = "nihai_veri_ğüşıöç.txt";
        let payload_file_path = cur_dir.join(terminal_file);
        let rel_file_path = rel_dir.join(terminal_file);

        let content = b"VOLTERRA_DEEP_25_LEVEL_TURKISH_UNICODE_PAYLOAD_SUCCESS";
        fs::write(&payload_file_path, content)?;
        let digest = Blake3Hasher::hash_to_digest(content);

        self.expectations.insert(
            rel_file_path.clone(),
            FileExpectation {
                relative_path: rel_file_path,
                size_bytes: content.len() as u64,
                blake3_digest: digest,
                is_sparse: false,
                max_allocated_blocks: None,
            },
        );

        // Also add intermediate files at level 5
        let mid5_rel = PathBuf::from(components[0])
            .join(components[1])
            .join(components[2])
            .join(components[3])
            .join(components[4])
            .join("seviye_05_ara_dosya.dat");
        let mid5_path = self.root.join(&mid5_rel);
        let mid5_content = b"SEVIYE_5_ARA_PAYLOAD";
        fs::write(&mid5_path, mid5_content)?;
        self.expectations.insert(
            mid5_rel.clone(),
            FileExpectation {
                relative_path: mid5_rel,
                size_bytes: mid5_content.len() as u64,
                blake3_digest: Blake3Hasher::hash_to_digest(mid5_content),
                is_sparse: false,
                max_allocated_blocks: None,
            },
        );

        Ok(self)
    }

    // ========================================================================
    // TEST-10: 1 GiB Sparse File with Unallocated Holes
    // ========================================================================
    pub fn add_sparse_file(
        &mut self,
        rel_filename: &str,
        logical_size: u64,
        extents: &[(u64, &[u8])],
    ) -> Result<&mut Self, std::io::Error> {
        let file_path = self.root.join(rel_filename);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&file_path)?;

        // 1. Set logical file length to 1 GiB (creates unallocated holes by default on APFS)
        file.set_len(logical_size)?;

        // 2. Write only non-zero extents
        for (offset, data) in extents {
            file.seek(SeekFrom::Start(*offset))?;
            file.write_all(data)?;
        }
        file.sync_all()?;

        // 3. Compute accurate full-file BLAKE3 hash by streaming the 1 GiB logical image
        file.seek(SeekFrom::Start(0))?;
        let digest = Blake3Hasher::hash_reader(&mut file)?;

        let meta = file.metadata()?;
        let allocated_blocks = meta.blocks();

        self.expectations.insert(
            PathBuf::from(rel_filename),
            FileExpectation {
                relative_path: PathBuf::from(rel_filename),
                size_bytes: logical_size,
                blake3_digest: digest,
                is_sparse: true,
                max_allocated_blocks: Some(allocated_blocks + 32), // allowance for APFS cluster overhead
            },
        );

        Ok(self)
    }
}

// ============================================================================
// APFS Sparse File Inspector & Extent Query Utility
// ============================================================================
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparseExtent {
    pub offset: u64,
    pub length: u64,
    pub is_data: bool,
}

pub struct SparseInspector;

impl SparseInspector {
    /// Detects whether file is sparse on Darwin APFS (st_blocks * 512 < st_size)
    pub fn is_sparse(path: &Path) -> Result<bool, std::io::Error> {
        let meta = fs::metadata(path)?;
        let logical_size = meta.len();
        let allocated_bytes = meta.blocks() * 512;
        Ok(logical_size > 64 * 1024 && allocated_bytes < logical_size.saturating_sub(64 * 1024))
    }

    /// Queries actual data and hole extents using Darwin lseek SEEK_DATA and SEEK_HOLE
    pub fn query_extents(path: &Path) -> Result<Vec<SparseExtent>, std::io::Error> {
        let file = File::open(path)?;
        let fd = file.as_raw_fd();
        let meta = file.metadata()?;
        let total_size = meta.len() as libc::off_t;

        let mut extents = Vec::new();
        let mut current_offset: libc::off_t = 0;

        while current_offset < total_size {
            let data_start = unsafe { libc::lseek(fd, current_offset, DARWIN_SEEK_DATA) };
            if data_start < 0 {
                if current_offset < total_size {
                    extents.push(SparseExtent {
                        offset: current_offset as u64,
                        length: (total_size - current_offset) as u64,
                        is_data: false,
                    });
                }
                break;
            }

            if data_start > current_offset {
                extents.push(SparseExtent {
                    offset: current_offset as u64,
                    length: (data_start - current_offset) as u64,
                    is_data: false,
                });
            }

            let hole_start = unsafe { libc::lseek(fd, data_start, DARWIN_SEEK_HOLE) };
            let data_end = if hole_start < 0 || hole_start > total_size {
                total_size
            } else {
                hole_start
            };

            extents.push(SparseExtent {
                offset: data_start as u64,
                length: (data_end - data_start) as u64,
                is_data: true,
            });

            current_offset = data_end;
        }

        Ok(extents)
    }
}

// ============================================================================
// Synthetic Dataset Deep Validator
// ============================================================================
#[derive(Debug)]
pub struct ValidationSummary {
    pub files_matched: usize,
    pub total_bytes_verified: u64,
    pub mismatches: Vec<String>,
}

pub struct SyntheticDatasetValidator;

impl SyntheticDatasetValidator {
    /// Validates a copied destination directory tree against expected file specifications.
    pub fn validate(
        target_root: &Path,
        expectations: &HashMap<PathBuf, FileExpectation>,
    ) -> Result<ValidationSummary, String> {
        let mut matched = 0;
        let mut total_bytes = 0u64;
        let mut mismatches = Vec::new();

        for (rel_path, exp) in expectations {
            let target_path = target_root.join(rel_path);
            if !target_path.exists() {
                mismatches.push(format!("Missing expected file: {}", rel_path.display()));
                continue;
            }

            let meta = match fs::metadata(&target_path) {
                Ok(m) => m,
                Err(e) => {
                    mismatches.push(format!("Cannot stat {}: {}", rel_path.display(), e));
                    continue;
                }
            };

            // 1. Length check
            if meta.len() != exp.size_bytes {
                mismatches.push(format!(
                    "Size mismatch for {}: expected {}, got {}",
                    rel_path.display(),
                    exp.size_bytes,
                    meta.len()
                ));
                continue;
            }

            // 2. BLAKE3 Digest check
            let mut file = match File::open(&target_path) {
                Ok(f) => f,
                Err(e) => {
                    mismatches.push(format!("Cannot open {}: {}", rel_path.display(), e));
                    continue;
                }
            };

            let actual_digest = if exp.size_bytes == 0 {
                Blake3Hasher::hash_to_digest(b"")
            } else {
                match Blake3Hasher::hash_reader(&mut file) {
                    Ok(d) => d,
                    Err(e) => {
                        mismatches.push(format!("Hash read error for {}: {}", rel_path.display(), e));
                        continue;
                    }
                }
            };

            if actual_digest != exp.blake3_digest {
                mismatches.push(format!(
                    "Checksum mismatch for {}: expected {}, got {}",
                    rel_path.display(),
                    exp.blake3_digest.to_hex(),
                    actual_digest.to_hex()
                ));
                continue;
            }

            // 3. Sparse preservation check
            if exp.is_sparse {
                let actual_blocks = meta.blocks();
                if let Some(max_blocks) = exp.max_allocated_blocks {
                    if actual_blocks > max_blocks {
                        mismatches.push(format!(
                            "Sparse file exploded on destination {}: expected max {} blocks, got {} blocks ({} bytes)",
                            rel_path.display(),
                            max_blocks,
                            actual_blocks,
                            actual_blocks * 512
                        ));
                        continue;
                    }
                }
            }

            matched += 1;
            total_bytes += exp.size_bytes;
        }

        if !mismatches.is_empty() {
            Err(format!(
                "Validation failed with {} mismatches:\n{}",
                mismatches.len(),
                mismatches.join("\n")
            ))
        } else {
            Ok(ValidationSummary {
                files_matched: matched,
                total_bytes_verified: total_bytes,
                mismatches,
            })
        }
    }
}
