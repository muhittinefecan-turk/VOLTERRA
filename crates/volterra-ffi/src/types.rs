//! C-compatible enums, structs, and function pointer types for the VOLTERRA C ABI.

use std::os::raw::c_char;

/// Transfer operation mode selector.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum volterra_mode {
    Turbo = 0,
    SmartHybrid = 1,
    SafeRecovery = 2,
}
pub type volterra_mode_t = volterra_mode;

/// Lifecycle execution state of a transfer job.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum volterra_state {
    Pending = 0,
    Scanning = 1,
    Running = 2,
    Paused = 3,
    Completed = 4,
    Failed = 5,
    WaitingForDisk = 6,
    Cancelled = 7,
}
pub type volterra_state_t = volterra_state;
pub type volterra_job_state_t = volterra_state;

/// Bottleneck diagnosis classification code.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum volterra_bottleneck {
    Nominal = 0,
    SourceRead = 1,
    DestWrite = 2,
    Interface = 3,
    Metadata = 4,
    SmallFile = 5,
    CpuIntegrity = 6,
    Contention = 7,
    ErrorDegradation = 8,
    Unknown = 9,
}
pub type volterra_bottleneck_t = volterra_bottleneck;

/// Event kind identifier delivered through asynchronous event subscription.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum volterra_event_kind {
    StateChanged = 1,
    ScanningProgress = 2,
    ScanningComplete = 3,
    FileStarted = 4,
    ChunkProgress = 5,
    FileCompleted = 6,
    FileQuarantined = 7,
    TelemetryUpdated = 8,
    BottleneckDetected = 9,
    DiskWarning = 10,
    JobCompleted = 11,
    JobFailed = 12,
}
pub type volterra_event_kind_t = volterra_event_kind;

/// Standard status and error codes returned across the C ABI.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum volterra_error_code {
    Ok = 0,
    ErrInvalidArgument = 1,
    ErrSourceNotFound = 2,
    ErrTargetNotFound = 3,
    ErrTargetExists = 4,
    ErrInsufficientSpace = 5,
    ErrPermissionDenied = 6,
    ErrReadIo = 7,
    ErrWriteIo = 8,
    ErrIntegrityMismatch = 9,
    ErrDiskDisconnected = 10,
    ErrSourceModified = 11,
    ErrUnsupportedMetadata = 12,
    ErrInvalidState = 13,
    ErrRecoveryFailed = 14,
    ErrInternalPanic = 99,
}
pub type volterra_error_code_t = volterra_error_code;

/// Output format for export report generation.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum volterra_report_format {
    Json = 0,
    Csv = 1,
}
pub type volterra_report_format_t = volterra_report_format;

/// Opaque transfer job controller handle.
pub struct volterra_job_t {
    pub handle: volterra_core::JobHandle,
    pub event_thread: parking_lot::Mutex<Option<std::thread::JoinHandle<()>>>,
    pub stop_signal: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Detailed error information passed as an out-parameter.
#[repr(C)]
pub struct volterra_error_t {
    pub code: i32,
    pub message: *mut c_char,
}

/// Configuration structure passed to `volterra_job_create`.
#[repr(C)]
pub struct volterra_config_t {
    pub source_path: *const c_char,
    pub dest_path: *const c_char,
    pub mode: u32,
    pub chunk_size: usize,
    pub direct_io: bool,
    pub verify_readback: bool,
    pub memory_budget: usize,
    pub allow_overwrite: bool,
    pub journal_path: *const c_char,
}

/// Real-time progress and status snapshot.
#[repr(C)]
pub struct volterra_snapshot_t {
    pub job_id: *mut c_char,
    pub state: u32,
    pub total_files: u64,
    pub processed_files: u64,
    pub failed_files: u64,
    pub skipped_files: u64,
    pub total_bytes: u64,
    pub processed_bytes: u64,
    pub speed_bps: f64,
    pub rolling_speed_bps: f64,
    pub eta_seconds: u64,
    pub error_count: u64,
    pub percent_complete: f64,
    pub read_latency_p50_ms: f64,
    pub read_latency_p95_ms: f64,
    pub write_latency_p50_ms: f64,
    pub write_latency_p95_ms: f64,
    pub active_chunk_size_bytes: usize,
    pub active_concurrency: usize,
    pub active_bottleneck: *mut c_char,
}

/// Fine-grained I/O latency, throughput, and scheduler telemetry.
#[repr(C)]
pub struct volterra_telemetry_t {
    pub ewma_1s_mbps: f64,
    pub ewma_5s_mbps: f64,
    pub ewma_30s_mbps: f64,
    pub instantaneous_mbps: f64,
    pub source_read_mbps: f64,
    pub dest_write_mbps: f64,
    pub physical_throughput_mbps: f64,
    pub p50_read_latency_ms: f64,
    pub p95_read_latency_ms: f64,
    pub p50_write_latency_ms: f64,
    pub p95_write_latency_ms: f64,
    pub buffer_utilization_pct: f64,
    pub active_chunk_size_bytes: usize,
    pub active_concurrency: usize,
    pub bottleneck_code: u32,
    pub bottleneck_message: *mut c_char,
    pub is_cache_burst: bool,
    pub is_stalled: bool,
    pub total_bytes_read: u64,
    pub total_bytes_written: u64,
    pub read_errors: u64,
    pub write_errors: u64,
}

/// Discrete failed logical extent within a file.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct volterra_extent_t {
    pub offset: u64,
    pub length: u64,
    pub error_code: i32,
}

/// Information regarding a single quarantined or failing file.
#[repr(C)]
pub struct volterra_error_item_t {
    pub file_id: i64,
    pub path: *mut c_char,
    pub error_kind: i32,
    pub error_message: *mut c_char,
    pub total_size_bytes: u64,
    pub state: u32,
    pub attempts: u32,
    pub failed_extents_count: usize,
    pub extents: *mut volterra_extent_t,
}

/// Array container of quarantined error items.
#[repr(C)]
pub struct volterra_error_list_t {
    pub count: usize,
    pub items: *mut volterra_error_item_t,
}

/// Physical storage device metadata and capabilities.
#[repr(C)]
pub struct volterra_disk_t {
    pub device_id: *mut c_char,
    pub bsd_name: *mut c_char,
    pub mount_point: *mut c_char,
    pub volume_name: *mut c_char,
    pub fs_type: *mut c_char,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub is_rotational: bool,
    pub is_removable: bool,
    pub is_apfs_synthesized: bool,
    pub parent_bsd_name: *mut c_char,
    pub model: *mut c_char,
    pub vendor: *mut c_char,
    pub transport: *mut c_char,
}

/// Discovered disk volume list.
#[repr(C)]
pub struct volterra_disk_list_t {
    pub count: usize,
    pub disks: *mut volterra_disk_t,
}

/// Event notification delivered to registered C callbacks.
#[repr(C)]
pub struct volterra_event_t {
    pub event_type: u32,
    pub file_id: i64,
    pub progress_bytes: u64,
    pub total_bytes: u64,
    pub message: *const c_char,
}

/// C callback signature for event notifications.
pub type volterra_event_callback_fn =
    Option<unsafe extern "C" fn(event: *const volterra_event_t, user_data: *mut libc::c_void)>;
