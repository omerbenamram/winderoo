# Rust Port Plan: esp-hal + Embassy (no_std async)

## Goal
Port the firmware to a pure-Rust HAL + async stack, prioritizing control, efficiency, and long-term portability over speed of initial delivery.

## Why this plan
- Minimal runtime overhead and fine-grained control.
- Full async concurrency with predictable scheduling.
- Easier to reuse for other MCUs later.

## Phases

### 1) Project skeleton + build
- Convert firmware crate to `no_std` with optional `std` for host tests.
- Add `esp-hal`, `esp-wifi`, `embassy-*` crates.
- Provide `embassy` executor setup and task scheduling.

### 2) Hardware layer
- Implement GPIO + PWM motor control and LED control via `esp-hal`.
- Add I2C OLED driver wiring (likely `embedded-graphics` + SSD1306).
- Create async-friendly abstractions.

### 3) Networking stack
- Bring up Wi-Fi with `esp-wifi`.
- Use `smoltcp` for TCP/IP.
- Implement HTTP server (embedded HTTP or custom minimal server).
- Implement mDNS (may require extra crate or manual UDP responder).

### 4) Time + RTC
- Implement SNTP client (custom or crate-based).
- Maintain epoch tracking + GMT/DST offsets in firmware.

### 5) Filesystem + settings
- Decide on storage: LittleFS via a crate, or custom NVS/flash region.
- Implement JSON serialization with `serde_json` (requires `alloc`).
- Persist and reload settings at boot.

### 6) HTTP API
- Implement endpoints to match existing frontend / OpenAPI.
- Reuse existing Rust API parsing + controller logic.

### 7) OLED UI + scheduling
- Port static/dynamic UI rendering with `embedded-graphics`.
- Implement screen sleep schedule with async timers.

### 8) Home Assistant MQTT (optional)
- Add MQTT client (async) via a crate.
- Map HA entity values to controller state.

### 9) Validation + tests
- Keep host tests for logic (std build).
- Add embedded test harness for peripheral behavior.
- On-device smoke test matrix.

## Risks / notes
- More engineering work and moving parts.
- Requires integrating multiple crates that aren’t as plug‑and‑play as ESP‑IDF.
- Some features (mDNS, filesystem, MQTT) require extra effort or custom implementations.

## Deliverables
- Fully async Rust firmware with a clean hardware abstraction layer.
- Documentation for build + flashing.
- Integration test harness for embedded targets.
