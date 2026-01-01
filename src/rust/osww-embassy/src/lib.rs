//! Pure Rust HAL wiring for the Winderoo firmware.
//!
//! This crate binds the portable firmware logic (`winderoo-firmware`) to
//! embedded-hal/embassy-based drivers. The goal is to keep IO-specific code
//! isolated while reusing the well-tested winding logic from the core crate.

#![no_std]

extern crate alloc;

pub mod hardware;
pub mod runtime;
pub mod tasks;
