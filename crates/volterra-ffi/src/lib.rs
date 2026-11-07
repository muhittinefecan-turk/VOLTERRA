//! VOLTERRA C ABI / Swift Concurrency FFI Bridge Library.
//!
//! Provides a stable, C99-compliant interface to the VOLTERRA core engine,
//! disk discovery, adaptive optimizer, recovery controls, and journaling.

#![allow(non_camel_case_types)]
#![allow(clippy::missing_safety_doc)]

pub mod bridge;
pub mod export;
pub mod memory;
pub mod types;

pub use bridge::*;
pub use export::*;
pub use memory::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_string() {
        unsafe {
            let ver = volterra_version();
            assert!(!ver.is_null());
            let s = std::ffi::CStr::from_ptr(ver).to_str().unwrap();
            assert_eq!(s, "1.0.0");
        }
    }

    #[test]
    fn test_header_generation() {
        let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let config_path = std::path::Path::new(&crate_dir).join("cbindgen.toml");
        let config = cbindgen::Config::from_file(&config_path).expect("cbindgen.toml missing");

        let header_path = std::path::Path::new(&crate_dir)
            .join("../../apps/VolterraMac/Sources/Bridge/volterra_ffi.h");

        if let Some(parent) = header_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let builder = cbindgen::Builder::new()
            .with_crate(&crate_dir)
            .with_config(config);

        let bindings = builder
            .generate()
            .expect("cbindgen failed to generate bindings");
        bindings.write_to_file(&header_path);

        assert!(header_path.exists());
        let content = std::fs::read_to_string(&header_path).unwrap();
        assert!(content.contains("VOLTERRA_FFI_H"));
        assert!(content.contains("volterra_job_create"));
        assert!(content.contains("volterra_get_snapshot"));
        assert!(content.contains("volterra_discover_disks"));
    }
}
