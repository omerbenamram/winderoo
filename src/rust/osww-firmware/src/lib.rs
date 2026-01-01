//! Winderoo firmware logic, data models, and controller utilities.
//!
//! This crate focuses on portable, testable logic extracted from the ESP32
//! firmware so the Rust port can be validated on a host machine. The
//! `esp32` feature is reserved for device-specific runtime wiring.

pub mod api;
pub mod controller;
pub mod hardware;
pub mod model;
pub mod settings;
pub mod time;

#[cfg(feature = "esp32")]
pub mod esp32;
