//! Logging abstraction that works with both defmt and tracing.
//!
//! This module provides logging macros that can use either `defmt` (for embedded targets)
//! or `tracing` (for desktop/testing) depending on the enabled cargo features.
//!
//! ## Features
//!
//! - `defmt`: Use defmt for logging
//! - `tracing`: Use tracing for logging
//! - Neither: Logging is compiled out (no-op)
//!
//! ## Usage
//!
//! ```rust,ignore
//! use crate::log::{trace, debug, info, warn, error};
//!
//! info!("Application started");
//! debug!("Value: {}", 42);
//! warn!("Something unexpected: {:?}", some_value);
//! ```

// Re-export Format trait when using defmt
#[cfg(feature = "defmt")]
pub use defmt::Format;

// For tracing or no logging, we provide a stub Format trait
#[allow(unused)]
#[cfg(not(feature = "defmt"))]
pub trait Format {}

// When using defmt, also provide Debug2Format for std types
#[cfg(feature = "defmt")]
pub use defmt::Debug2Format;

// For tracing or no logging, Debug2Format is a passthrough
#[allow(non_snake_case)]
#[cfg(not(feature = "defmt"))]
#[inline]
pub fn Debug2Format<T>(value: &T) -> &T {
    value
}

// Logging macros that dispatch to the appropriate backend or no-op
// If both features are enabled, defmt takes precedence

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {
        #[cfg(feature = "defmt")]
        defmt::trace!($($arg)*);

        #[cfg(all(feature = "tracing", not(feature = "defmt")))]
        tracing::trace!($($arg)*);

        #[cfg(not(any(feature = "defmt", feature = "tracing")))]
        { let _ = format_args!($($arg)*); } // no-op, format_args! borrows without moving
    };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        #[cfg(feature = "defmt")]
        defmt::debug!($($arg)*);

        #[cfg(all(feature = "tracing", not(feature = "defmt")))]
        tracing::debug!($($arg)*);

        #[cfg(not(any(feature = "defmt", feature = "tracing")))]
        { let _ = format_args!($($arg)*); } // no-op, format_args! borrows without moving
    };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        #[cfg(feature = "defmt")]
        defmt::info!($($arg)*);

        #[cfg(all(feature = "tracing", not(feature = "defmt")))]
        tracing::info!($($arg)*);

        #[cfg(not(any(feature = "defmt", feature = "tracing")))]
        { let _ = format_args!($($arg)*); } // no-op, format_args! borrows without moving
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        #[cfg(feature = "defmt")]
        defmt::warn!($($arg)*);

        #[cfg(all(feature = "tracing", not(feature = "defmt")))]
        tracing::warn!($($arg)*);

        #[cfg(not(any(feature = "defmt", feature = "tracing")))]
        { let _ = format_args!($($arg)*); } // no-op, format_args! borrows without moving
    };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        #[cfg(feature = "defmt")]
        defmt::error!($($arg)*);

        #[cfg(all(feature = "tracing", not(feature = "defmt")))]
        tracing::error!($($arg)*);

        #[cfg(not(any(feature = "defmt", feature = "tracing")))]
        { let _ = format_args!($($arg)*); } // no-op, format_args! borrows without moving
    };
}

// Re-export the macros at the module level for easier use
#[allow(unused)]
pub use crate::{debug, error, info, trace, warn};
