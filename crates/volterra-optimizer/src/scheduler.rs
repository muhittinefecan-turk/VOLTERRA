use crate::chunk_engine::ChunkSize;
use serde::{Deserialize, Serialize};
use volterra_profiler::{DiskRelationship, TransferDirection};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerProfile {
    pub reader_concurrency: usize,
    pub writer_concurrency: usize,
    pub queue_depth: usize,
    pub read_ahead_mb: usize,
    pub serialized_batching: bool,
    pub batch_buffer_bytes: usize,
    pub initial_chunk_size: ChunkSize,
    pub rationale: String,
}

pub struct IoScheduler;

impl IoScheduler {
    /// Generates an intelligent I/O scheduler profile matching the discovered media relationship.
    pub fn create_profile(
        relationship: &DiskRelationship,
        memory_budget_bytes: usize,
    ) -> SchedulerProfile {
        match relationship.direction {
            TransferDirection::HddToSsd => SchedulerProfile {
                reader_concurrency: 1, // Strict single reader to avoid head thrashing
                writer_concurrency: 1,
                queue_depth: 3,
                read_ahead_mb: 32, // Bounded read-ahead advisory via F_RDADVISE
                serialized_batching: false,
                batch_buffer_bytes: 0,
                initial_chunk_size: ChunkSize::MiB8,
                rationale: "HDD to SSD: Sequential single reader with Darwin read-ahead and async SSD writer.".into(),
            },
            TransferDirection::SsdToHdd => SchedulerProfile {
                reader_concurrency: 1,
                writer_concurrency: 1, // Single sequential writer prevents rotational fragmentation
                queue_depth: 2,
                read_ahead_mb: 8,
                serialized_batching: false,
                batch_buffer_bytes: 0,
                initial_chunk_size: ChunkSize::MiB8,
                rationale: "SSD to HDD: Sequential single writer preventing mechanical seek storms.".into(),
            },
            TransferDirection::SsdToSsd => {
                let max_workers = (memory_budget_bytes / (32 * 1024 * 1024)).clamp(2, 4);
                SchedulerProfile {
                    reader_concurrency: max_workers,
                    writer_concurrency: max_workers,
                    queue_depth: max_workers * 2,
                    read_ahead_mb: 64,
                    serialized_batching: false,
                    batch_buffer_bytes: 0,
                    initial_chunk_size: ChunkSize::MiB16,
                    rationale: "SSD to SSD: Multi-worker parallel I/O saturating NVMe and APFS bus channels.".into(),
                }
            }
            TransferDirection::HddToHdd => SchedulerProfile {
                reader_concurrency: 1,
                writer_concurrency: 1,
                queue_depth: 3,
                read_ahead_mb: 16,
                serialized_batching: false,
                batch_buffer_bytes: 0,
                initial_chunk_size: ChunkSize::MiB8,
                rationale: "HDD to HDD: Decoupled sequential reader and writer on distinct physical spindles.".into(),
            },
            TransferDirection::SameDiskTransfer => {
                // Determine whether same physical disk is rotational or SSD
                let is_rotational = relationship.warning.as_ref().is_some_and(|w| w.contains("rotational"));
                if is_rotational {
                    // Critical safety policy: Serialized batching to eliminate head thrashing
                    let batch_size = (memory_budget_bytes / 2).clamp(16 * 1024 * 1024, 64 * 1024 * 1024);
                    SchedulerProfile {
                        reader_concurrency: 1,
                        writer_concurrency: 1,
                        queue_depth: 1,
                        read_ahead_mb: 0,
                        serialized_batching: true,
                        batch_buffer_bytes: batch_size,
                        initial_chunk_size: ChunkSize::MiB8,
                        rationale: "Same Physical HDD: Serialized batching pipeline reading full batches before writing.".into(),
                    }
                } else {
                    SchedulerProfile {
                        reader_concurrency: 2,
                        writer_concurrency: 2,
                        queue_depth: 3,
                        read_ahead_mb: 16,
                        serialized_batching: false,
                        batch_buffer_bytes: 0,
                        initial_chunk_size: ChunkSize::MiB8,
                        rationale: "Same Physical SSD: Conservative concurrency sharing physical NAND controller.".into(),
                    }
                }
            }
            TransferDirection::Unknown => SchedulerProfile {
                reader_concurrency: 1,
                writer_concurrency: 1,
                queue_depth: 2,
                read_ahead_mb: 8,
                serialized_batching: false,
                batch_buffer_bytes: 0,
                initial_chunk_size: ChunkSize::MiB8,
                rationale: "Unknown media: Conservative balanced defaults with adaptive feedback tuning.".into(),
            },
        }
    }
}
