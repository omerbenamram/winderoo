//! Winderoo firmware logic, data models, and controller utilities.
//!
//! This crate focuses on portable, testable logic extracted from the ESP32
//! firmware so the Rust port can be validated on a host machine. The core
//! modules compile in `no_std` environments (with `alloc`) so they can be
//! reused by a pure Rust HAL runtime.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod api;
pub mod controller;
pub mod hardware;
pub mod model;
pub mod settings;
pub mod time;
