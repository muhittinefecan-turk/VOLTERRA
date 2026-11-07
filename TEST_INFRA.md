# VOLTERRA Test Infrastructure Specification (`TEST_INFRA.md`)

## 1. Architectural Principles
1. **Hermetic & Isolated**: Tests operate strictly within temporary directories (`tempfile::tempdir()`). No host user files are read or modified.
2. **Zero Production Mutation**: Source paths are opened read-only. Destination paths enforce collision checks and atomic replacement.
3. **Deterministic Fault Injection**: `ReadFaultInjector` and `WriteFaultInjector` simulate exact POSIX error codes (`EIO`, `ENOSPC`, `EACCES`, `ENODEV`) and byte-range short reads without requiring hardware manipulation.
4. **Reproducible Benchmarking**: TEST-11 provides calibrated medium latency models and real disk I/O modes.

## 2. Directory Layout & Workspace Integration
- `tests/Cargo.toml`: Package `volterra-test-suite` linked in root `Cargo.toml`.
- `tests/src/lib.rs`: Common harness, test environment, synthetic fixtures, fault injection hooks, and medium models.
- `tests/src/fixtures/synthetic.rs`: Boundary files (TEST-07), 10,000 small files (TEST-08), 25-level deep Turkish Unicode trees (TEST-09), 1 GiB APFS sparse files (TEST-10).
- `tests/src/fault_injection/`: Readers/writers simulating sector failures (`EIO`, short reads, `ENOSPC`, `EACCES`, process crash WAL replay, `ENODEV` disconnects).
- `tests/e2e/`: 4-tier E2E test suites and TEST-11 benchmark:
  - `tier1_feature_coverage.rs`: Full feature coverage across all 10 core subsystems.
  - `tier2_boundaries.rs`: Boundary limits, RAM budget backpressure, preflight guards, deep trees, pause/resume stress.
  - `tier3_cross_feature.rs`: Error quarantine, FFI remediation, closed-loop optimizer error fallback, SIGKILL WAL recovery, collision sheet swap, readback corruption barriers.
  - `tier4_realworld_scenarios.rs`: Multi-gigabyte heterogeneous disk migration, degraded disk rescue with sidecars, USB disconnect & volume UUID reconnection, dynamic adaptive scaling.
  - `test_fixed_vs_adaptive.rs`: TEST-11 calibrated benchmark comparing Fixed 1 MiB vs Fixed 8 MiB vs Adaptive Dynamic 1-64 MiB.

## 3. Systematic 4-Tier Test Matrix
- **Tier 1 (Feature Coverage)**: Happy path across all 10 subsystems (Manifest, Engine, Journal, Integrity, Core, FFI, Profiler).
- **Tier 2 (Boundary & Corner Cases)**: 0B/1B/chunk boundary files, memory budget bounding (backpressure), capacity preflight, 25-level Unicode trees, rapid lifecycle controls.
- **Tier 3 (Cross-Feature Combinations)**: Error isolation with FFI skip/retry, optimizer error fallback to 1 MiB, WAL reconciliation on SIGKILL crash, collision sheet swap, readback corruption barriers.
- **Tier 4 (Real-World Application Scenarios)**: Heterogeneous disk copy (photos/videos/repos), degraded disk salvage with sidecar manifests, USB disconnect chaos, multi-GB optimizer scaling.

## 4. Execution Commands
```bash
# Run all workspace tests (unit + integration + E2E across all 11 crates)
cargo test --workspace

# Run only E2E test suites
cargo test -p volterra-test-suite

# Run Tier 1 Feature Coverage
cargo test -p volterra-test-suite --test tier1_feature_coverage

# Run Tier 2 Boundaries
cargo test -p volterra-test-suite --test tier2_boundaries

# Run Tier 3 Cross-Feature
cargo test -p volterra-test-suite --test tier3_cross_feature

# Run Tier 4 Real-World Scenarios
cargo test -p volterra-test-suite --test tier4_realworld_scenarios

# Run TEST-11 Adaptive Optimizer Benchmark with formatted output
cargo test -p volterra-test-suite --test test_fixed_vs_adaptive -- --nocapture
```
