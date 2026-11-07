use crate::error::RecoveryError;
use crate::extent::{ExtentInterval, ExtentMap};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoleFillPolicy {
    ZeroFilled,
    SparseHole,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarManifest {
    pub volterra_sidecar_version: u32,
    pub job_id: String,
    pub relative_path: PathBuf,
    pub total_size_bytes: u64,
    pub recovered_bytes: u64,
    pub unreadable_bytes: u64,
    pub total_missing_bytes: u64,
    pub total_recovered_bytes: u64,
    pub recovery_percentage: f64,
    pub is_complete: bool,
    pub is_partial_recovery: bool,
    pub checksum_verified: bool,
    pub hole_fill_policy: HoleFillPolicy,
    pub missing_extents: Vec<ExtentInterval>,
    pub healthy_extents: Vec<ExtentInterval>,
    pub recovered_extents: Vec<ExtentInterval>,
    pub created_at_utc: String,
    pub warning: String,
}

pub type PartialRecoverySidecar = SidecarManifest;

impl SidecarManifest {
    /// Creates a partial recovery sidecar manifest.
    /// Strictly enforces fail-closed invariants:
    /// - is_complete is ALWAYS false
    /// - is_partial_recovery is ALWAYS true
    /// - checksum_verified is ALWAYS false
    pub fn create(
        job_id: String,
        relative_path: PathBuf,
        total_size: u64,
        failed_extents: &ExtentMap,
        hole_fill_policy: HoleFillPolicy,
    ) -> Self {
        let missing = failed_extents.intervals().to_vec();
        let unreadable_bytes = failed_extents.total_bytes();
        let healthy = failed_extents.invert(total_size);
        let recovered_bytes = healthy.iter().map(|i| i.length).sum();

        let recovery_pct = if total_size > 0 {
            (recovered_bytes as f64 / total_size as f64) * 100.0
        } else {
            0.0
        };

        Self {
            volterra_sidecar_version: 1,
            job_id,
            relative_path,
            total_size_bytes: total_size,
            recovered_bytes,
            unreadable_bytes,
            total_missing_bytes: unreadable_bytes,
            total_recovered_bytes: recovered_bytes,
            recovery_percentage: recovery_pct,
            is_complete: false,
            is_partial_recovery: true,
            checksum_verified: false,
            hole_fill_policy,
            missing_extents: missing,
            healthy_extents: healthy.clone(),
            recovered_extents: healthy,
            created_at_utc: chrono::Utc::now().to_rfc3339(),
            warning: "WARNING: This file is an incomplete recovery. Unreadable sectors could not be read from source media. DO NOT treat as a complete or bit-identical copy.".to_string(),
        }
    }

    /// Writes the sidecar JSON manifest to the designated disk path.
    pub fn write_to_file(&self, path: &Path) -> Result<(), RecoveryError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let json = serde_json::to_string_pretty(self)?;
        let mut file = File::create(path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    /// Reads and parses a sidecar JSON manifest from disk.
    pub fn read_from_file(path: &Path) -> Result<Self, RecoveryError> {
        let mut file = File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        let manifest: Self = serde_json::from_str(&contents)?;
        Ok(manifest)
    }

    /// Strictly enforces the fail-closed publication barrier.
    /// Returns Err if the file is incomplete, has missing extents, or failed verification.
    pub fn enforce_fail_closed_barrier(
        total_size: u64,
        verified_bytes: u64,
        missing_extents_count: usize,
    ) -> Result<(), RecoveryError> {
        if verified_bytes < total_size || missing_extents_count > 0 {
            return Err(RecoveryError::FailClosedBarrierFailed {
                total_size,
                verified_bytes,
                missing_extents: missing_extents_count,
            });
        }
        Ok(())
    }
}
