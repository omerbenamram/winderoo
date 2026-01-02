//! Pure Rust HAL wiring for the Winderoo firmware.
//!
//! This crate binds the portable firmware logic (`winderoo-firmware`) to
//! embedded-hal/embassy-based drivers. The goal is to keep IO-specific code
//! isolated while reusing the well-tested winding logic from the core crate.

#![no_std]

extern crate alloc;

pub mod hardware;
pub mod captive_portal;
pub mod http;
#[cfg(feature = "esp32")]
pub mod esp32;
pub mod sntp;
pub mod state;
pub mod system;
pub mod tasks;
pub mod time;
pub mod wifi;
