# VOLTERRA Workspace Rules & Engineering Invariants

## 1. Non-Destructive Data Safety Invariants (Mandatory)
- **Read-Only Source**: Never open source files with write permissions, and never delete or modify source files or their metadata under any normal, recovery, or fault-injection scenario.
- **Explicit Overwrite Consent**: Never overwrite existing destination files without explicit user approval (`RENAME_EXCL` by default; `RENAME_SWAP` only when explicitly authorized). In SwiftUI (`TransferViewModel`), `allowOverwriteConfirmed` MUST be synchronously reset to `false` on every job reset, cancellation, error abort, URL change, or completion.
- **Temporary `.partial` Isolation & 5-Step Atomic Publish**: All transfers must write to `.volterra.<jobid>.partial` files and publish only after: (1) `sync_all`, (2) BLAKE3 chunk & readback verification (`F_NOCACHE`), (3) source `inode`/`dev`/`mtime_ns` TOCTOU revalidation, (4) metadata application, and (5) `renamex_np` atomic rename.
- **Fail-Closed Partial Files**: Files with unreadable extents (`EIO`) must never be marked `COMPLETED`; write `.recovered.partial` with a `.recovered.partial.json` sidecar manifest recording exact `[offset, length)` missing ranges.

## 2. APFS & macOS Filesystem Rules
- **Sparse File Hole Preservation**: When writing chunks in `volterra-transfer`, detect all-zero buffers (`buf.iter().all(|&b| b == 0)`) and advance file length via `set_len` (plus 4096-byte block hole coalescing) rather than calling `pwrite` with zero bytes, preserving unallocated APFS sparse holes (`st_blocks * 512`) while still updating the BLAKE3 digest stream.
- **Canonical Mount Matching**: Normalize Darwin volume paths (`/System/Volumes/Data/...` vs `/Volumes/...`) before matching disk arbitration and mount events.

## 3. Adaptive Disk Profiler (ADP) & Optimizer (ITO) Rules
- **No Unmeasured Claims**: Never infer disk speed from model names or interface strings; report unsupported hardware metrics as `Unknown`.
- **Error-Aware Throttling**: Immediately disable aggressive concurrency or active benchmarking when `EIO` or device stalls occur, falling back to sequential safe recovery mode.
- **Synthetic Verification First**: All failure modes (`EIO`, `ENOSPC`, `EACCES`, short reads, `SIGKILL` crash recovery, `ENODEV` disconnect) must be tested via `ReadFaultInjector` / `WriteFaultInjector` and synthetic fixtures before touching physical media.

## 4. C ABI / FFI & SwiftUI Concurrency Rules
- **Panic Shielding & Ownership**: Wrap every `extern "C"` entry point in `std::panic::catch_unwind`. Memory allocated in Rust must only be freed via dedicated `volterra_*_free` functions.
- **Strict State Machine Guards**: `volterra_job_resume` must only succeed from `Paused` or `WaitingForDisk` states, returning `ErrInvalidState` (13) for `Pending` or terminal states.
- **60Hz UI Throttling & Handle Unbinding**: Stream FFI events via `DisplayThrottler` (60Hz normal lane, immediate critical lane) so disk I/O never blocks `MainActor`. Always unbind `ErrorCenterViewModel` from active job handles upon job termination.
