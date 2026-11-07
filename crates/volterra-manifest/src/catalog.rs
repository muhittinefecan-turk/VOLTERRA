use crate::model::{ChunkSpec, ItemType, Manifest, ManifestItem};
use std::path::Path;
use volterra_integrity::Blake3Hasher;

pub struct ManifestCatalog;

impl ManifestCatalog {
    /// Sorts files to optimize sequential access on rotational HDDs.
    /// Groups by parent directory and then sorts by inode to minimize head seeks.
    pub fn order_for_hdd(manifest: &Manifest) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..manifest.items.len()).collect();
        indices.sort_by(|&a, &b| {
            let item_a = &manifest.items[a];
            let item_b = &manifest.items[b];

            let parent_a = item_a.relative_path.parent();
            let parent_b = item_b.relative_path.parent();

            parent_a
                .cmp(&parent_b)
                .then_with(|| item_a.inode.cmp(&item_b.inode))
        });
        indices
    }

    /// Sorts files for concurrent SSD transfer.
    /// Orders by size descending so large files start first and maximize concurrent bus bandwidth.
    pub fn order_for_ssd(manifest: &Manifest) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..manifest.items.len()).collect();
        indices.sort_by(|&a, &b| {
            let item_a = &manifest.items[a];
            let item_b = &manifest.items[b];

            item_b
                .size_bytes
                .cmp(&item_a.size_bytes)
                .then_with(|| item_a.relative_path.cmp(&item_b.relative_path))
        });
        indices
    }

    /// Generates chunk specifications for a manifest item.
    /// Guards against `chunk_size == 0` to prevent divide-by-zero panics.
    pub fn generate_chunks_for_file(item: &ManifestItem, chunk_size: usize) -> Vec<ChunkSpec> {
        if chunk_size == 0 || item.item_type != ItemType::RegularFile {
            return Vec::new();
        }

        let size = item.size_bytes;
        if size == 0 {
            // Empty file: 1 zero-length chunk record with dynamically computed BLAKE3 hash
            return vec![ChunkSpec {
                file_id: item.id,
                chunk_index: 0,
                offset_bytes: 0,
                length_bytes: 0,
                digest: Some(Blake3Hasher::hash_to_digest(b"").to_hex()),
            }];
        }

        let chunk_size = chunk_size as u64;
        let num_chunks = size.div_ceil(chunk_size);
        let mut chunks = Vec::with_capacity(num_chunks as usize);

        for i in 0..num_chunks {
            let offset = i * chunk_size;
            let length = std::cmp::min(chunk_size, size - offset) as u32;
            chunks.push(ChunkSpec {
                file_id: item.id,
                chunk_index: i as u32,
                offset_bytes: offset,
                length_bytes: length,
                digest: None,
            });
        }

        chunks
    }

    /// Computes total chunk count across all files in the catalog.
    /// Returns 0 if `chunk_size == 0` to prevent divide-by-zero panics.
    pub fn compute_total_chunks(manifest: &Manifest, chunk_size: usize) -> u64 {
        if chunk_size == 0 {
            return 0;
        }

        let chunk_size = chunk_size as u64;
        let mut total = 0;
        for item in &manifest.items {
            if item.item_type == ItemType::RegularFile {
                if item.size_bytes == 0 {
                    total += 1;
                } else {
                    total += item.size_bytes.div_ceil(chunk_size);
                }
            }
        }
        total
    }

    /// Finds item by relative path in manifest.
    pub fn find_by_path<'a>(manifest: &'a Manifest, rel_path: &Path) -> Option<&'a ManifestItem> {
        let clean = rel_path.to_string_lossy();
        let trimmed = clean.trim_start_matches('/');
        manifest
            .items
            .iter()
            .find(|i| i.relative_path.to_string_lossy().trim_start_matches('/') == trimmed)
    }
}
