use parking_lot::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct ChunkBufferPool {
    chunk_size: usize,
    max_buffers: usize,
    in_flight_buffers: AtomicUsize,
    pool: Mutex<Vec<Vec<u8>>>,
}

impl ChunkBufferPool {
    pub fn new(chunk_size: usize, memory_budget_bytes: usize) -> Self {
        let max_buffers = std::cmp::max(1, memory_budget_bytes / chunk_size);
        Self {
            chunk_size,
            max_buffers,
            in_flight_buffers: AtomicUsize::new(0),
            pool: Mutex::new(Vec::with_capacity(max_buffers)),
        }
    }

    pub fn acquire(&self) -> Vec<u8> {
        self.in_flight_buffers.fetch_add(1, Ordering::Relaxed);
        let mut pool = self.pool.lock();
        pool.pop().unwrap_or_else(|| vec![0u8; self.chunk_size])
    }

    pub fn release(&self, mut buf: Vec<u8>) {
        self.in_flight_buffers.fetch_sub(1, Ordering::Relaxed);
        if buf.capacity() >= self.chunk_size {
            buf.resize(self.chunk_size, 0);
            let mut pool = self.pool.lock();
            if pool.len() < self.max_buffers {
                pool.push(buf);
            }
        }
    }

    pub fn in_flight_count(&self) -> usize {
        self.in_flight_buffers.load(Ordering::Relaxed)
    }

    pub fn max_buffers(&self) -> usize {
        self.max_buffers
    }

    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    pub fn utilization_pct(&self) -> f64 {
        let in_flight = self.in_flight_buffers.load(Ordering::Relaxed);
        if self.max_buffers > 0 {
            (in_flight as f64 / self.max_buffers as f64) * 100.0
        } else {
            0.0
        }
    }
}
