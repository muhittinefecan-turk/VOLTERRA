# Project: VOLTERRA macOS Smart Disk Copy & Data Recovery (v1.0 Release)
## Adaptive Disk Profiler (ADP) & Intelligent Transfer Optimizer (ITO) Extended

## Architecture
VOLTERRA is structured into two decoupled tracks:
1. **Implementation Track**: A modular, multi-crate Rust core workspace exposing a clean C ABI via `volterra-ffi`, consumed by a native SwiftUI macOS desktop application via an asynchronous Swift concurrency bridge.
2. **E2E Testing Track**: An independent, requirement-driven, opaque-box test suite featuring synthetic datasets, fault-injecting mock layers (`FaultInjectingReader`, `FaultInjectingWriter`) simulating `EIO`, short reads, `ENOSPC`, `EACCES`, `SIGKILL` crash, and USB unmount, verifying all acceptance criteria.

```
+--------------------------------------------------------------------------------------------------------------------+
|                                           Native macOS App (SwiftUI)                                               |
|  Dashboard & Profiler  |  Live Transfer & Charts  |  Bottleneck Monitor  |  Error Center  |  Job History & Reports |
+--------------------------------------------------------------------------------------------------------------------+
                                                          │
                                       AsyncStream<VolterraBridgeEvent> (60Hz throttled)
                                                          ▼
+--------------------------------------------------------------------------------------------------------------------+
|                                    Swift Concurrency Bridge (apps/VolterraMac/Bridge)                              |
+--------------------------------------------------------------------------------------------------------------------+
                                                          │
                                        C ABI FFI (volterra-ffi / volterra_ffi.h)
                                        Universal Binary (arm64 + x86_64 fat lib)
                                                          ▼
+--------------------------------------------------------------------------------------------------------------------+
|                                                   volterra-core                                                    |
|                         TransferController, JobLifecycle, EventHub, JobSnapshot, Telemetry                         |
+--------------------------------------------------------------------------------------------------------------------+
        │                        │                          │                         │                     │
        ▼                        ▼                          ▼                         ▼                     ▼
+----------------+       +----------------+         +----------------+        +---------------+     +---------------+
|volterra-manifest|       |volterra-transfer|        |volterra-recovery|      |volterra-journal|    |volterra-prof  |
| Scanner, Stats |       | Reader, Writer,|         | Error Queue,   |        | SQLite WAL,   |    | Disk Discovery|
| Symlink Handler|       | Bounded Queue, |         | Extent Maps,   |        | Jobs, Files,  |    | EWMA Latencies|
| Source Checks  |       | Atomic Publish |         | Skip/Retry/All |        | Chunks, Errors|    | Safe Benchmark|
+----------------+       +----------------+         +----------------+        +---------------+     +---------------+
        │                        │                          │                         │                     │
        └────────────────────────┼──────────────────────────┴─────────────────────────┼─────────────────────┘
                                 ▼                                                    ▼
                     +-----------------------+                            +-----------------------+
                     |  volterra-optimizer   |                            |  volterra-platform-mac|
                     |  Closed-Loop Feedback |                            |  IOKit, DiskArbitrat. |
                     |  Adaptive Chunk Engine|                            |  statfs, fcntl direct |
                     |  Bottleneck Detector  |                            |  xattr, setattrlist   |
                     |  HDD/SSD Schedulers   |                            |  renamex_np atomic    |
                     +-----------------------+                            +-----------------------+
                                 │                                                    │
                                 └────────────────────────┬───────────────────────────┘
                                                          ▼
                                              +-----------------------+
                                              |  volterra-integrity   |
                                              |  BLAKE3 256-bit hash  |
                                              |  Chunk & File digests |
                                              |  Readback verify      |
                                              +-----------------------+
```

## Feature Inventory
| # | Feature ID | Name & Description | Milestone | Source |
|---|---|---|---|---|
| 1 | BLD-01 | Cargo Workspace root configuration linking crates | M1 | Survey |
| 2 | PLAT-01 | `statfs` filesystem capacity, block size & free space query | M1 | Survey |
| 3 | PLAT-02 | Persistent volume UUID extraction via `getattrlist(ATTR_VOL_UUID)` | M1 | Survey |
| 4 | PLAT-03 | Media rotation detection (IOKit query + adaptive latency profiling) | M1 | Survey |
| 5 | PLAT-04 | Direct I/O cache control (`fcntl` F_NOCACHE, F_GLOBAL_NOCACHE) | M1 | Survey |
| 6 | PLAT-05 | Darwin read-ahead advisory (`fcntl` F_RDADVISE, F_RDAHEAD) | M1 | Survey |
| 7 | PLAT-06 | Non-destructive atomic replacement (`renamex_np` RENAME_EXCL / RENAME_SWAP) | M1 | Survey |
| 8 | PLAT-07 | Extended attributes preservation (Darwin 6-arg `xattr`, `XATTR_NOFOLLOW`) | M1 | Survey |
| 9 | PLAT-08 | Creation time (`btime`) & modification time (`mtime`) via `setattrlist` | M1 | Survey |
| 10 | PLAT-09 | POSIX file modes, permissions & ownership staging (`fchmod`, `fchown`) | M1 | Survey |
| 11 | PLAT-10 | Symlink detection and preservation (`readlink`, `symlink`) without cycle loops | M1 | Survey |
| 12 | INTEG-01 | BLAKE3 256-bit chunk hashing (8 MiB / adaptive chunks) | M1 | Survey |
| 13 | INTEG-02 | BLAKE3 whole-file digest aggregation | M1 | Survey |
| 14 | INTEG-03 | Optional physical readback verification from target storage media | M1 | Survey |
| 15 | INTEG-04 | Digest verification before publication (corrupt chunks never completed) | M1 | Survey |
| 16 | MANI-01 | Recursive directory tree scanner with metadata extraction | M1 | Survey |
| 17 | MANI-02 | File catalog ordering and total payload computation | M1 | Survey |
| 18 | MANI-03 | Source mutation detection (re-validate size/mtime before publication) | M1 | Survey |
| 19 | JOUR-01 | SQLite WAL mode initialization (`PRAGMA journal_mode = WAL`) | M2 | Survey |
| 20 | JOUR-02 | Normalized relational schema (`jobs`, `files`, `chunks`, `errors` tables) | M2 | Survey |
| 21 | JOUR-03 | Batched transaction commit & checkpointing | M2 | Survey |
| 22 | JOUR-04 | Crash recovery reconciliation (reconstruct state from WAL upon launch) | M2 | Survey |
| 23 | JOUR-05 | Resumption chunk reuse (preserve verified chunks in `.partial` files) | M2 | Survey |
| 24 | RECO-01 | Error classification (`EIO`, `ENOSPC`, `EACCES`, `ENODEV`, short reads) | M2 | Survey |
| 25 | RECO-02 | Logical extent error recording (`[offset_bytes, length_bytes)`) in SQLite | M2 | Survey |
| 26 | RECO-03 | Error queue isolation (quarantine unreadable files, continue healthy files) | M2 | Survey |
| 27 | RECO-04 | Interactive per-file controls (`Skip`, `Retry`, `Skip All`, `Resume`) | M2 | Survey |
| 28 | RECO-05 | Partial file recovery (`.recovered.partial` with missing extent sidecar) | M2 | Survey |
| 29 | RECO-06 | Retry budgets, jittered backoff, and max-retry thresholds | M2 | Survey |
| 30 | XFER-01 | Reader worker using POSIX `pread` (`FileExt`) and safe read loops | M3 | Survey |
| 31 | XFER-02 | Writer worker using POSIX `pwrite` (`FileExt`) to `.volterra.<jobid>.partial` | M3 | Survey |
| 32 | XFER-03 | Bounded memory buffer channels enforcing strict RAM backpressure | M3 | Survey |
| 33 | XFER-04 | Adaptive scheduler (HDD single sequential reader, SSD concurrency) | M3 | Survey |
| 34 | XFER-05 | Transfer profiling warmup (p50/p95 latency tracking, dynamic chunk sizing) | M3 | Survey |
| 35 | XFER-06 | Safe temporary file lifecycle (`.partial` -> sync -> atomic replace) | M3 | Survey |
| 36 | XFER-07 | Pre-publication OS buffer flushing (`sync_all`) | M3 | Survey |
| 37 | XFER-08 | Pre-flight capacity & read-only target validation | M3 | Survey |
| 38 | XFER-09 | Overwrite collision protection (prohibit silent overwrites) | M3 | Survey |
| 39 | CORE-01 | Job lifecycle state machine (`Pending`, `Scanning`, `Running`, `Paused`, `Completed`, `Failed`, `WaitingForDisk`, `Cancelled`) | M3 | Survey |
| 40 | CORE-02 | `JobBuilder` configuration (modes: Turbo, Smart Hybrid, Safe Recovery) | M3 | Survey |
| 41 | CORE-03 | `EventHub` non-blocking async event broadcast channel | M3 | Survey |
| 42 | CORE-04 | `JobHandle` thread-safe lifecycle control handle | M3 | Survey |
| 43 | CORE-05 | `JobSnapshot` telemetry snapshot generation | M3 | Survey |
| 44 | PROF-01 | Automatic Disk Discovery (DiskArbitration, IOKit, POSIX, connection types, multi-volume) | M3 | ADP/ITO Follow-up |
| 45 | PROF-02 | Real-Time Performance Monitor (EWMA 1s, 5s, 30s throughput, read/write latencies) | M3 | ADP/ITO Follow-up |
| 46 | PROF-03 | Physical vs Cache Throughput Discriminator (avoid filesystem cache illusions) | M3 | ADP/ITO Follow-up |
| 47 | PROF-04 | Safe Disk Benchmarking (Passive default + opt-in non-destructive temp file benchmark) | M3 | ADP/ITO Follow-up |
| 48 | OPT-01 | Adaptive Chunk Engine (Candidate chunks: 1, 4, 8, 16, 32, 64 MiB; backpressure) | M3 | ADP/ITO Follow-up |
| 49 | OPT-02 | Intelligent I/O Scheduler (HDD->SSD, SSD->HDD, SSD->SSD, HDD->HDD policies) | M3 | ADP/ITO Follow-up |
| 50 | OPT-03 | Automatic Bottleneck Detection (Source-read, Dest-write, Interface, Metadata, Contention) | M3 | ADP/ITO Follow-up |
| 51 | OPT-04 | Smart Adaptive Optimizer (Closed-loop feedback controller, anti-oscillation) | M3 | ADP/ITO Follow-up |
| 52 | OPT-05 | Error-Aware Adaptive Mode (Fallback to conservative recovery upon error spikes) | M3 | ADP/ITO Follow-up |
| 53 | FFI-01 | `volterra_job_create` exported C ABI symbol | M4 | Survey |
| 54 | FFI-02 | `volterra_job_start` exported C ABI symbol | M4 | Survey |
| 55 | FFI-03 | `volterra_job_pause` exported C ABI symbol | M4 | Survey |
| 56 | FFI-04 | `volterra_job_resume` exported C ABI symbol | M4 | Survey |
| 57 | FFI-05 | `volterra_job_cancel` exported C ABI symbol | M4 | Survey |
| 58 | FFI-06 | `volterra_job_free` exported C ABI symbol | M4 | Survey |
| 59 | FFI-07 | `volterra_get_snapshot` exported C ABI symbol | M4 | Survey |
| 60 | FFI-08 | `volterra_subscribe_events` (non-blocking async C callback) | M4 | Survey |
| 61 | FFI-09 | `volterra_get_errors` exported C ABI symbol | M4 | Survey |
| 62 | FFI-10 | `volterra_skip_file` exported C ABI symbol | M4 | Survey |
| 63 | FFI-11 | `volterra_retry_file` exported C ABI symbol | M4 | Survey |
| 64 | FFI-12 | `volterra_defer_file` exported C ABI symbol | M4 | Survey |
| 65 | FFI-13 | `volterra_skip_all_errors` exported C ABI symbol | M4 | Survey |
| 66 | FFI-14 | `volterra_retry_all_failed` exported C ABI symbol | M4 | Survey |
| 67 | FFI-15 | `volterra_export_report` (JSON and CSV formats) | M4 | Survey |
| 68 | FFI-16 | Dedicated Rust memory deallocators (`volterra_string_free`, etc.) | M4 | Survey |
| 69 | FFI-17 | C ABI header generation via `cbindgen` (`volterra_ffi.h`) | M4 | Survey |
| 70 | FFI-18 | Strict panic boundary shielding (`std::panic::catch_unwind`) | M4 | Survey |
| 71 | FFI-19 | FFI Telemetry & Optimizer Structures (`volterra_telemetry_t`, bottleneck flags) | M4 | ADP/ITO Follow-up |
| 72 | BLD-02 | Universal macOS Binary build pipeline (`scripts/build-universal.sh`, `lipo`) | M4 | Survey |
| 73 | BLD-03 | Clang module map & C bridging (`module.modulemap`) | M4 | Survey |
| 74 | UI-01 | SwiftUI application structure & lifecycle (`VolterraMacApp`) | M5 | Survey |
| 75 | UI-02 | Swift `VolterraBridge` wrapper around C ABI | M5 | Survey |
| 76 | UI-03 | `AsyncStream<VolterraBridgeEvent>` event pipeline | M5 | Survey |
| 77 | UI-04 | `@MainActor` ViewModel with 60Hz UI update throttling | M5 | Survey |
| 78 | UI-05 | Dashboard: Source and Target volume/folder selection with mount metadata | M5 | Survey |
| 79 | UI-06 | Dashboard: Copy mode selector (Automatic, Performance, Balanced, Recovery) | M5 | Survey & ADP/ITO |
| 80 | UI-07 | Dashboard: Pre-flight capacity checks & APFS->exFAT metadata warnings | M5 | Survey |
| 81 | UI-08 | Dashboard: Destination collision warning sheet (explicit approval required) | M5 | Survey |
| 82 | UI-09 | Live Transfer: Real-time progress bar, bytes copied, file counts | M5 | Survey |
| 83 | UI-10 | Live Transfer: Instantaneous & smoothed throughput (MB/s), dynamic ETA | M5 | Survey |
| 84 | UI-11 | Live Transfer: Read/Write p50 & p95 latency monitor | M5 | Survey |
| 85 | UI-12 | Live Transfer: Pause, Resume, and Cancel interactive controls | M5 | Survey |
| 86 | UI-13 | Performance Dashboard: Side-by-side Source & Destination device panels | M5 | ADP/ITO Follow-up |
| 87 | UI-14 | Performance Dashboard: Real-time rolling throughput charts (1s, 5s, 30s) | M5 | ADP/ITO Follow-up |
| 88 | UI-15 | Performance Dashboard: Bottleneck indicators & active chunk engine badge | M5 | ADP/ITO Follow-up |
| 89 | UI-16 | Error Center: Tabular error list showing unreadable/failed files | M5 | Survey |
| 90 | UI-17 | Error Center: Failed extent inspector (byte offsets, lengths, errno) | M5 | Survey |
| 91 | UI-18 | Error Center: Interactive per-file `Skip`, `Retry`, and `Later` actions | M5 | Survey |
| 92 | UI-19 | Error Center: Bulk `Skip All` and batch remediation actions | M5 | Survey |
| 93 | UI-20 | Job History: Historical transfer run browser backed by SQLite journal | M5 | Survey |
| 94 | UI-21 | Job History: Report export dialogs for JSON and CSV | M5 | Survey |
| 95 | TEST-01 | Fault Injection: `FaultInjectingReader` simulating `EIO` sector errors | M-Final / E2E | Survey |
| 96 | TEST-02 | Fault Injection: Short read simulation loop verification | M-Final / E2E | Survey |
| 97 | TEST-03 | Fault Injection: `FaultInjectingWriter` simulating `ENOSPC` | M-Final / E2E | Survey |
| 98 | TEST-04 | Fault Injection: Permission denial (`EACCES`) handling | M-Final / E2E | Survey |
| 99 | TEST-05 | Fault Injection: Abrupt process crash / `SIGKILL` WAL replay | M-Final / E2E | Survey |
| 100 | TEST-06 | Fault Injection: Abrupt USB disconnect (`ENODEV`) volume UUID handling | M-Final / E2E | Survey |
| 101 | TEST-07 | Synthetic Data: Boundary edge cases (0B, 1B, 8MiB boundaries) | M-Final / E2E | Survey |
| 102 | TEST-08 | Synthetic Data: 10,000 small files deep traversal workload | M-Final / E2E | Survey |
| 103 | TEST-09 | Synthetic Data: 25-level deep directory tree & Unicode/Turkish paths | M-Final / E2E | Survey |
| 104 | TEST-10 | Synthetic Data: 1 GiB sparse file with unallocated holes | M-Final / E2E | Survey |
| 105 | TEST-11 | Adaptive Optimizer Benchmark: Fixed vs Adaptive comparison benchmark | M-Final / E2E | ADP/ITO Follow-up |

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| M1 | Platform, Integrity & Manifest Foundations | `volterra-platform-macos`, `volterra-integrity`, `volterra-manifest`, Root `Cargo.toml`. Darwin direct I/O, rotation detection, atomic replace, xattr/btime/mtime, BLAKE3 chunk/file hashing, manifest scanner. | None | DONE |
| M2 | SQLite WAL Journaling & Error Recovery Engine | `volterra-journal`, `volterra-recovery`. Normalized schema, WAL durability, crash reconciliation; extent-based error tracking, error queue isolation, interactive Skip/Retry/Skip All controls, partial file tracking. | None (Interfaces with M1) | DONE |
| M3 | Transfer Pipeline, Adaptive Profiler & Intelligent Optimizer | `volterra-transfer`, `volterra-profiler`, `volterra-optimizer`, `volterra-core`. Reader/Writer workers, bounded memory channels, automatic disk discovery, real-time EWMA telemetry, adaptive chunk engine (1–64 MiB), bottleneck detector, closed-loop feedback optimizer, HDD/SSD scheduling policies, job controller. | M1, M2 | IN_PROGRESS |
| M4 | C ABI FFI Bridge & Universal Build Pipeline | `volterra-ffi`, `scripts/build-universal.sh`. Complete C ABI exported symbols, telemetry & optimizer structs, `cbindgen` header generation, panic shielding, non-blocking event callback, fat binary compilation (`arm64` + `x86_64` via `lipo`). | M1, M2, M3 | PLANNED |
| M5 | Native macOS SwiftUI Desktop Application & Performance Dashboard | `apps/VolterraMac`. Swift concurrency bridge (`VolterraBridge`), 60Hz throttled ViewModel, Dashboard with Source/Dest panels, Live Transfer View with charts, Bottleneck indicators, Error Center with extent inspector, Job History with JSON/CSV export. | M4 | PLANNED |
| M-Final | Final Milestone: 100% E2E Pass & Adversarial Hardening | Phase 1: Pass 100% E2E test suite (Tiers 1-4) published by E2E Testing Track + Optimizer comparison benchmark. Phase 2: Adversarial coverage hardening (Tier 5) with Challenger. | M1, M2, M3, M4, M5, TEST_READY.md | PLANNED |

## Interface Contracts

### 1. `volterra-platform-macos` ↔ `volterra-transfer` & `volterra-profiler`
- `PlatformFs::query_stats(path: &Path) -> Result<VolumeStats, PlatformError>`
- `PlatformVolume::get_volume_uuid(path: &Path) -> Result<VolumeUuid, PlatformError>`
- `PlatformProfile::detect_media_rotation(path: &Path) -> Result<MediaRotation, PlatformError>`
- `PlatformIo::set_direct_io(fd: RawFd) -> Result<(), PlatformError>`
- `PlatformIo::advise_readahead(fd: RawFd, offset: off_t, length: off_t) -> Result<(), PlatformError>`
- `PlatformAtomic::atomic_replace(source_partial: &Path, target_dest: &Path, allow_overwrite: bool) -> Result<(), PlatformError>`
- `PlatformMetadata::copy_metadata(src: &Path, dst: &Path) -> Result<MetadataPreserved, PlatformError>`

### 2. `volterra-integrity` ↔ `volterra-transfer`
- `Blake3Hasher::hash_chunk(buffer: &[u8]) -> [u8; 32]`
- `Blake3FileHasher::update(&mut self, chunk_digest: &[u8; 32], offset: u64, len: usize)`
- `Blake3FileHasher::finalize(self) -> [u8; 32]`
- `ReadbackVerifier::verify_range(fd: RawFd, offset: u64, expected_digest: &[u8; 32], len: usize) -> Result<bool, IntegrityError>`

### 3. `volterra-journal` ↔ `volterra-core` & `volterra-recovery`
- `JournalEngine::open(db_path: &Path) -> Result<Self, JournalError>`
- `JournalEngine::create_job(&self, job: &JobRecord) -> Result<(), JournalError>`
- `JournalEngine::record_files_batch(&self, job_id: &str, files: &[FileRecord]) -> Result<(), JournalError>`
- `JournalEngine::record_chunk_verified(&self, chunk: &ChunkRecord) -> Result<(), JournalError>`
- `JournalEngine::record_extent_error(&self, error: &ExtentErrorRecord) -> Result<(), JournalError>`
- `JournalEngine::update_file_status(&self, file_id: i64, status: FileStatus) -> Result<(), JournalError>`
- `JournalEngine::load_job_state(&self, job_id: &str) -> Result<JobRecoveryState, JournalError>`
- `JournalEngine::get_verified_chunks(&self, file_id: i64) -> Result<Vec<ChunkRecord>, JournalError>`

### 4. `volterra-profiler` ↔ `volterra-optimizer` & `volterra-core`
- `DiskDiscoveryEngine::discover_disks() -> Result<Vec<DiskDeviceProfile>, ProfilerError>`
- `PerformanceMonitor::record_read(&self, bytes: usize, latency_ns: u64)`
- `PerformanceMonitor::record_write(&self, bytes: usize, latency_ns: u64)`
- `PerformanceMonitor::get_telemetry() -> TransferTelemetry`
  - Returns `ewma_1s_mbps`, `ewma_5s_mbps`, `ewma_30s_mbps`, `p50_read_latency_ms`, `p95_read_latency_ms`, `p50_write_latency_ms`, `p95_write_latency_ms`, `buffer_utilization_pct`.

### 5. `volterra-optimizer` ↔ `volterra-transfer` & `volterra-core`
- `TransferOptimizer::evaluate_step(&mut self, telemetry: &TransferTelemetry) -> OptimizationDecision`
  - Recommends: `chunk_size_bytes` (1, 4, 8, 16, 32, or 64 MiB), `queue_depth`, `concurrency`, `read_ahead_mb`.
- `TransferOptimizer::detect_bottleneck(&self, telemetry: &TransferTelemetry) -> BottleneckClassification`
- `TransferOptimizer::set_policy(&mut self, policy: TransferPolicy)`

### 6. `volterra-ffi` ↔ Swift Concurrency Bridge
- Exported C ABI symbols with strict pointer ownership, telemetry extensions, and `catch_unwind` safety:
  ```c
  volterra_job_t* volterra_job_create(const volterra_config_t* config, volterra_error_t** out_err);
  int volterra_job_start(volterra_job_t* job, volterra_error_t** out_err);
  int volterra_job_pause(volterra_job_t* job, volterra_error_t** out_err);
  int volterra_job_resume(volterra_job_t* job, volterra_error_t** out_err);
  int volterra_job_cancel(volterra_job_t* job, volterra_error_t** out_err);
  void volterra_job_free(volterra_job_t* job);
  volterra_snapshot_t* volterra_get_snapshot(volterra_job_t* job);
  volterra_telemetry_t* volterra_get_telemetry(volterra_job_t* job);
  void volterra_telemetry_free(volterra_telemetry_t* tel);
  void volterra_snapshot_free(volterra_snapshot_t* snap);
  int volterra_subscribe_events(volterra_job_t* job, volterra_event_callback_fn cb, void* user_data);
  volterra_error_list_t* volterra_get_errors(volterra_job_t* job);
  void volterra_error_list_free(volterra_error_list_t* list);
  int volterra_skip_file(volterra_job_t* job, int64_t file_id);
  int volterra_retry_file(volterra_job_t* job, int64_t file_id);
  int volterra_skip_all_errors(volterra_job_t* job);
  char* volterra_export_report(volterra_job_t* job, int format_type);
  void volterra_string_free(char* str);
  void volterra_error_free(volterra_error_t* err);
  ```

## Code Layout
```text
/Users/muhittinefecanturk/Documents/antigravity/VOLTERRA/
├── Cargo.toml
├── crates/
│   ├── volterra-platform-macos/
│   │   ├── Cargo.toml
│   │   └── src/ (fs.rs, volume.rs, profile.rs, io.rs, atomic.rs, xattr.rs, time.rs, perms.rs, symlink.rs, lib.rs)
│   ├── volterra-integrity/
│   │   ├── Cargo.toml
│   │   └── src/ (blake3.rs, readback.rs, validator.rs, lib.rs)
│   ├── volterra-manifest/
│   │   ├── Cargo.toml
│   │   └── src/ (model.rs, scanner.rs, catalog.rs, metadata.rs, lib.rs)
│   ├── volterra-journal/
│   │   ├── Cargo.toml
│   │   └── src/ (db.rs, schema.rs, queries.rs, recovery.rs, lib.rs)
│   ├── volterra-recovery/
│   │   ├── Cargo.toml
│   │   └── src/ (classifier.rs, extent.rs, queue.rs, actions.rs, lib.rs)
│   ├── volterra-profiler/
│   │   ├── Cargo.toml
│   │   └── src/ (discovery.rs, monitor.rs, benchmark.rs, telemetry.rs, lib.rs)
│   ├── volterra-optimizer/
│   │   ├── Cargo.toml
│   │   └── src/ (controller.rs, chunk_engine.rs, scheduler.rs, bottleneck.rs, policy.rs, lib.rs)
│   ├── volterra-transfer/
│   │   ├── Cargo.toml
│   │   └── src/ (reader.rs, writer.rs, buffer.rs, scheduler.rs, profile.rs, atomic_publish.rs, lib.rs)
│   ├── volterra-core/
│   │   ├── Cargo.toml
│   │   └── src/ (controller.rs, job.rs, state.rs, events.rs, telemetry.rs, lib.rs)
│   └── volterra-ffi/
│       ├── Cargo.toml
│       ├── cbindgen.toml
│       └── src/ (types.rs, bridge.rs, memory.rs, export.rs, lib.rs)
├── apps/
│   └── VolterraMac/
│       ├── Package.swift
│       ├── Sources/
│       │   ├── App/ (VolterraMacApp.swift)
│       │   ├── Bridge/ (volterra_ffi.h, module.modulemap, VolterraBridge.swift)
│       │   ├── ViewModels/ (TransferViewModel.swift, ErrorCenterViewModel.swift, HistoryViewModel.swift, PerformanceViewModel.swift)
│       │   └── Views/ (DashboardView.swift, LiveTransferView.swift, PerformanceDashboardView.swift, ErrorCenterView.swift, HistoryView.swift, Components/)
│       └── Tests/
├── tests/
│   ├── fault_injection/ (reader.rs, writer.rs, crash.rs, disconnect.rs)
│   ├── fixtures/ (synthetic.rs)
│   └── e2e/ (test_fixed_vs_adaptive.rs)
└── scripts/
    └── build-universal.sh
```
