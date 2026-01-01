//! ESP32-specific runtime integration (feature-gated).
//!
//! This module is intentionally lightweight in the initial port. It provides
//! the entrypoint where ESP-IDF services (WiFi, HTTP server, FS, OLED, etc.)
//! will be wired to the controller. The core logic lives in the rest of the
//! crate and is fully testable on the host.

use thiserror::Error;

/// Errors that can occur in the ESP32 runtime layer.
#[derive(Debug, Error)]
pub enum Esp32Error {
    /// The ESP32 runtime wiring has not been implemented yet.
    #[error("esp32 runtime wiring not implemented")]
    NotImplemented,
}

/// Start the ESP32 runtime loop.
///
/// This is a placeholder for the device integration entrypoint.
pub fn run() -> Result<(), Esp32Error> {
    Err(Esp32Error::NotImplemented)
}
