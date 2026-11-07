use serde::{Deserialize, Serialize};
use volterra_journal::ChunkState;
use volterra_profiler::{TransferDirection, TransferTelemetry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(usize)]
pub enum ChunkSize {
    MiB1 = 1024 * 1024,
    MiB4 = 4 * 1024 * 1024,
    MiB8 = 8 * 1024 * 1024,
    MiB16 = 16 * 1024 * 1024,
    MiB32 = 32 * 1024 * 1024,
    MiB64 = 64 * 1024 * 1024,
}

impl ChunkSize {
    pub fn as_bytes(&self) -> usize {
        *self as usize
    }

    pub fn from_bytes(bytes: usize) -> Self {
        if bytes >= 64 * 1024 * 1024 {
            ChunkSize::MiB64
        } else if bytes >= 32 * 1024 * 1024 {
            ChunkSize::MiB32
        } else if bytes >= 16 * 1024 * 1024 {
            ChunkSize::MiB16
        } else if bytes >= 8 * 1024 * 1024 {
            ChunkSize::MiB8
        } else if bytes >= 4 * 1024 * 1024 {
            ChunkSize::MiB4
        } else {
            ChunkSize::MiB1
        }
    }

    pub fn step_up(&self) -> Self {
        match self {
            ChunkSize::MiB1 => ChunkSize::MiB4,
            ChunkSize::MiB4 => ChunkSize::MiB8,
            ChunkSize::MiB8 => ChunkSize::MiB16,
            ChunkSize::MiB16 => ChunkSize::MiB32,
            ChunkSize::MiB32 => ChunkSize::MiB64,
            ChunkSize::MiB64 => ChunkSize::MiB64,
        }
    }

    pub fn step_down(&self) -> Self {
        match self {
            ChunkSize::MiB64 => ChunkSize::MiB32,
            ChunkSize::MiB32 => ChunkSize::MiB16,
            ChunkSize::MiB16 => ChunkSize::MiB8,
            ChunkSize::MiB8 => ChunkSize::MiB4,
            ChunkSize::MiB4 => ChunkSize::MiB1,
            ChunkSize::MiB1 => ChunkSize::MiB1,
        }
    }

    pub fn all_candidates() -> &'static [ChunkSize] {
        &[
            ChunkSize::MiB1,
            ChunkSize::MiB4,
            ChunkSize::MiB8,
            ChunkSize::MiB16,
            ChunkSize::MiB32,
            ChunkSize::MiB64,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdaptiveChunk {
    pub file_id: i64,
    pub chunk_index: u32,
    pub offset_bytes: u64,
    pub length_bytes: u32,
    pub state: ChunkState,
}

pub struct AdaptiveChunkEngine;

impl AdaptiveChunkEngine {
    /// Selects optimal candidate chunk size based on observed throughput, latency,
    /// file size, memory budget, error status, and transfer direction.
    pub fn determine_chunk_size(
        telemetry: &TransferTelemetry,
        file_size: u64,
        memory_budget_bytes: usize,
        queue_depth: usize,
        error_detected: bool,
        direction: TransferDirection,
    ) -> ChunkSize {
        // Dimension 1: Error degradation immediately clamps chunk size to 1 MiB for fault isolation
        if error_detected || telemetry.read_errors > 0 || telemetry.write_errors > 0 {
            return ChunkSize::MiB1;
        }

        // Dimension 2: Memory budget bounding
        let qd = queue_depth.max(1);
        let max_allowed_by_budget = memory_budget_bytes / qd;
        let mut candidate = ChunkSize::from_bytes(max_allowed_by_budget);

        // Dimension 3: File size boundary
        let candidate_by_file = if file_size < 1024 * 1024 {
            ChunkSize::MiB1
        } else if file_size < 8 * 1024 * 1024 {
            ChunkSize::MiB4
        } else if file_size < 32 * 1024 * 1024 {
            ChunkSize::MiB8
        } else if file_size < 128 * 1024 * 1024 {
            ChunkSize::MiB16
        } else if file_size < 512 * 1024 * 1024 {
            ChunkSize::MiB32
        } else {
            ChunkSize::MiB64
        };
        candidate = candidate.min(candidate_by_file);

        // Dimension 4: Latency & Queueing pressure
        if telemetry.p95_read_latency_ms > 120.0 || telemetry.p95_write_latency_ms > 120.0 {
            candidate = candidate.step_down();
        }

        // Dimension 5: Throughput scaling
        let speed = telemetry.ewma_5s_mbps;
        let candidate_by_speed = if speed < 30.0 {
            ChunkSize::MiB4
        } else if speed < 150.0 {
            ChunkSize::MiB8
        } else if speed < 450.0 {
            ChunkSize::MiB16
        } else if telemetry.p95_read_latency_ms < 20.0 && telemetry.p95_write_latency_ms < 20.0 {
            ChunkSize::MiB64
        } else {
            ChunkSize::MiB32
        };

        let result = candidate.min(candidate_by_speed);

        // Rotational disk constraint: avoid excessive chunk sizes on same disk transfer
        if direction == TransferDirection::SameDiskTransfer && result > ChunkSize::MiB16 {
            ChunkSize::MiB16
        } else {
            result
        }
    }

    /// Slices an entire file into contiguous, monotonically indexed adaptive chunks.
    pub fn slice_file(file_id: i64, file_size: u64, chunk_size: ChunkSize) -> Vec<AdaptiveChunk> {
        if file_size == 0 {
            return vec![AdaptiveChunk {
                file_id,
                chunk_index: 0,
                offset_bytes: 0,
                length_bytes: 0,
                state: ChunkState::Pending,
            }];
        }

        let chunk_len = chunk_size.as_bytes() as u64;
        let mut chunks = Vec::new();
        let mut offset = 0u64;
        let mut index = 0u32;

        while offset < file_size {
            let remaining = file_size - offset;
            let len = remaining.min(chunk_len) as u32;

            chunks.push(AdaptiveChunk {
                file_id,
                chunk_index: index,
                offset_bytes: offset,
                length_bytes: len,
                state: ChunkState::Pending,
            });

            offset += len as u64;
            index += 1;
        }

        chunks
    }

    /// Slices uncopied missing extents into adaptive chunks for resumption, preserving verified chunks.
    pub fn slice_missing_extents(
        file_id: i64,
        missing_extents: &[(u64, u64)],
        chunk_size: ChunkSize,
    ) -> Vec<AdaptiveChunk> {
        let chunk_len = chunk_size.as_bytes() as u64;
        let mut chunks = Vec::new();
        let mut index = 0u32;

        for &(extent_offset, extent_len) in missing_extents {
            let mut offset = extent_offset;
            let end = extent_offset + extent_len;

            while offset < end {
                let remaining = end - offset;
                let len = remaining.min(chunk_len) as u32;

                chunks.push(AdaptiveChunk {
                    file_id,
                    chunk_index: index,
                    offset_bytes: offset,
                    length_bytes: len,
                    state: ChunkState::Pending,
                });

                offset += len as u64;
                index += 1;
            }
        }

        chunks
    }
}
