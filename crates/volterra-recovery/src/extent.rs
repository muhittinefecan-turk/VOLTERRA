use crate::error::RecoveryError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ExtentInterval {
    pub offset: u64,
    pub length: u64,
}

pub type Extent = ExtentInterval;

impl ExtentInterval {
    /// Creates a new extent interval [offset, offset + length).
    /// Returns Err if length is 0 or offset + length overflows u64.
    pub fn new(offset: u64, length: u64) -> Result<Self, RecoveryError> {
        if length == 0 {
            return Err(RecoveryError::InvalidExtent {
                offset,
                length,
                reason: "Extent length cannot be zero".to_string(),
            });
        }

        if offset.checked_add(length).is_none() {
            return Err(RecoveryError::InvalidExtent {
                offset,
                length,
                reason: "Extent offset + length exceeds u64 limit".to_string(),
            });
        }

        Ok(Self { offset, length })
    }

    /// Half-open end offset: offset + length
    pub fn end(&self) -> u64 {
        self.offset.saturating_add(self.length)
    }

    /// Checks if a single offset falls within this extent [offset, end)
    pub fn contains_offset(&self, offset: u64) -> bool {
        offset >= self.offset && offset < self.end()
    }

    /// Checks if this extent strictly overlaps with another
    pub fn overlaps(&self, other: &Self) -> bool {
        self.offset < other.end() && other.offset < self.end()
    }

    /// Checks if this extent is immediately adjacent/contiguous with another
    pub fn is_contiguous(&self, other: &Self) -> bool {
        self.end() == other.offset || other.end() == self.offset
    }

    /// Checks if this extent overlaps or is immediately adjacent to another
    pub fn overlaps_or_adjacent(&self, other: &Self) -> bool {
        self.offset <= other.end() && other.offset <= self.end()
    }

    /// Merges two overlapping or adjacent extents into a single bounding extent.
    /// Returns None if the extents are strictly disjoint.
    pub fn merge(&self, other: &Self) -> Option<Self> {
        if !self.overlaps_or_adjacent(other) {
            return None;
        }

        let start = self.offset.min(other.offset);
        let end = self.end().max(other.end());
        Some(Self {
            offset: start,
            length: end - start,
        })
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtentMap {
    intervals: Vec<ExtentInterval>,
}

pub type ExtentSet = ExtentMap;

impl ExtentMap {
    /// Creates an empty set of extent intervals.
    pub fn new() -> Self {
        Self {
            intervals: Vec::new(),
        }
    }

    /// Inserts an extent interval, automatically merging overlapping or contiguous ranges.
    pub fn insert(&mut self, extent: ExtentInterval) {
        if extent.length == 0 {
            return;
        }

        let mut current = extent;
        let mut new_intervals = Vec::with_capacity(self.intervals.len() + 1);
        let mut inserted = false;

        for existing in self.intervals.drain(..) {
            if current.overlaps_or_adjacent(&existing) {
                // Merge into current
                current = current.merge(&existing).unwrap();
            } else if !inserted && existing.offset > current.end() {
                // Insert merged current before this strictly greater interval
                new_intervals.push(current);
                new_intervals.push(existing);
                inserted = true;
            } else {
                new_intervals.push(existing);
            }
        }

        if !inserted {
            new_intervals.push(current);
        }

        self.intervals = new_intervals;
    }

    /// Adds an extent by offset and length, silently ignoring invalid/zero-length extents.
    pub fn add_extent(&mut self, offset: u64, length: u64) {
        if let Ok(ext) = ExtentInterval::new(offset, length) {
            self.insert(ext);
        }
    }

    /// Inserts an extent range, returning Err if length is 0 or overflows.
    pub fn insert_range(&mut self, offset: u64, length: u64) -> Result<(), RecoveryError> {
        let ext = ExtentInterval::new(offset, length)?;
        self.insert(ext);
        Ok(())
    }

    /// Returns the total sum of bytes covered by all disjoint intervals.
    pub fn total_bytes(&self) -> u64 {
        self.intervals.iter().map(|i| i.length).sum()
    }

    /// Alias for total_bytes.
    pub fn total_failed_bytes(&self) -> u64 {
        self.total_bytes()
    }

    /// Returns a slice of the sorted disjoint intervals.
    pub fn intervals(&self) -> &[ExtentInterval] {
        &self.intervals
    }

    /// Alias for intervals.
    pub fn extents(&self) -> &[ExtentInterval] {
        &self.intervals
    }

    /// Returns the number of disjoint intervals.
    pub fn count(&self) -> usize {
        self.intervals.len()
    }

    /// Returns true if the set contains no intervals.
    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty()
    }

    /// Inverts the extents against a total file size, returning the complement
    /// (e.g. readable slices from failed error extents).
    pub fn invert(&self, total_size: u64) -> Vec<ExtentInterval> {
        let mut inverted = Vec::new();
        let mut cursor = 0u64;

        for interval in &self.intervals {
            if interval.offset > cursor {
                let limit = interval.offset.min(total_size);
                if limit > cursor {
                    if let Ok(slice) = ExtentInterval::new(cursor, limit - cursor) {
                        inverted.push(slice);
                    }
                }
            }
            cursor = cursor.max(interval.end());
            if cursor >= total_size {
                break;
            }
        }

        if cursor < total_size {
            if let Ok(slice) = ExtentInterval::new(cursor, total_size - cursor) {
                inverted.push(slice);
            }
        }

        inverted
    }

    /// Returns an ExtentMap containing the complement intervals.
    pub fn complement(&self, total_size: u64) -> ExtentMap {
        ExtentMap {
            intervals: self.invert(total_size),
        }
    }
}
