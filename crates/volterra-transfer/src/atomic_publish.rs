use crate::error::TransferError;
use std::fs::File;
use std::path::Path;
use volterra_integrity::{IntegrityValidator, ReadbackVerifier};
use volterra_manifest::{ManifestItem, SourceValidator};
use volterra_platform_macos::{AtomicReplaceResult, PlatformAtomic, PlatformMetadata};

pub struct AtomicPublisher;

impl AtomicPublisher {
    /// Executes the 6-step non-destructive atomic publication workflow:
    /// 1. OS Buffer Flush (`sync_all` / `F_FULLFSYNC`)
    /// 2. Digest & Size Integrity Validation (with optional direct I/O readback)
    /// 3. Source Mutation Re-validation (mtime, size, inode)
    /// 4. Metadata Preservation (POSIX mode, mtime/btime, extended attributes)
    /// 5. Atomic Replacement (`renamex_np` with RENAME_EXCL / RENAME_SWAP)
    #[allow(clippy::too_many_arguments)]
    pub fn publish_file(
        temp_file: &File,
        temp_path: &Path,
        final_dest_path: &Path,
        source_root: &Path,
        source_item: &ManifestItem,
        expected_size: u64,
        expected_digest: Option<&[u8; 32]>,
        verify_readback: bool,
        allow_overwrite: bool,
    ) -> Result<AtomicReplaceResult, TransferError> {
        // Step 1: Pre-publication OS buffer flush
        temp_file
            .sync_all()
            .map_err(|e| TransferError::DestinationWriteError {
                offset: expected_size,
                len: 0,
                source: e,
            })?;

        // Step 2: Digest & size validation (with optional direct I/O readback)
        let verifier = ReadbackVerifier::new();
        let digest_to_verify = if verify_readback {
            expected_digest
        } else {
            None
        };
        IntegrityValidator::validate_file_for_publication(
            temp_file,
            Some(temp_path),
            expected_size,
            digest_to_verify,
            &verifier,
        )?;

        // Step 3: Source mutation re-validation
        let source_full_path = source_root.join(&source_item.relative_path);
        let mutation = SourceValidator::revalidate_source_mutation(source_item, &source_full_path)
            .map_err(|e| TransferError::SourceReadError {
                offset: 0,
                len: 0,
                source: std::io::Error::other(e.to_string()),
            })?;
        if !mutation.is_unmodified() {
            return Err(TransferError::SourceMutated(mutation));
        }

        // Step 4: Metadata preservation (POSIX mode, mtime, btime, xattrs)
        let _ = PlatformMetadata::copy_metadata(&source_full_path, temp_path);

        // Step 5: Atomic replacement using Darwin renamex_np
        let result = PlatformAtomic::atomic_replace(temp_path, final_dest_path, allow_overwrite)?;

        Ok(result)
    }
}
