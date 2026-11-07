//! VOLTERRA Test Suite Library (`volterra-test-suite`)
//!
//! Provides synthetic dataset builders, calibrated disk emulators, fault injection hooks,
//! and hermetic test environment fixtures across 4 tiers of E2E verification.

pub mod fault_injection;
pub mod fixtures;
pub mod harness;
pub mod medium_emulator;

pub use fault_injection::*;
pub use fixtures::*;
pub use harness::*;
pub use medium_emulator::*;
