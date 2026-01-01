# Rust Port Plan: ESP-IDF (esp-idf-svc)

## Goal
Port the existing ESP32 firmware to Rust using ESP-IDF services to preserve feature parity with the current Arduino/C++ implementation while minimizing rework.

## Why this plan
- Closest mapping to current functionality (Wi-Fi provisioning, HTTP server, mDNS, NTP/RTC, FS, MQTT).
- Most stable and documented path for ESP32 in Rust today.
- Fastest route to a working device with minimal behavior drift.

## Phases

### 1) Project skeleton + build
- Create ESP-IDF enabled binary target in `src/rust/osww-firmware`.
- Add `esp-idf-svc`, `esp-idf-hal`, `embedded-svc` wiring behind `esp32` feature.
- Provide `sdkconfig.defaults` (Wi-Fi, LwIP, mDNS, SNTP, LittleFS).
- Confirm `cargo esp32-build` works.

### 2) Hardware layer
- Implement hardware traits (GPIO for motor, PWM for LED, I2C OLED).
- Map to ESP-IDF HAL drivers.
- Add safe wrappers for `digitalWrite` equivalents and PWM channels.

### 3) Wi-Fi provisioning + captive portal
- Implement Wi-Fi setup with a captive portal (ESP-IDF HTTP server).
- Persist SSID/pass to NVS or a settings file.
- Handle auto-connect with fallback to AP mode.

### 4) Filesystem + settings
- Mount LittleFS (or FAT) at startup.
- Load settings JSON into `RuntimeState` using existing Rust models.
- Persist updates on API writes.

### 5) HTTP API
- Implement endpoints: `/api/status`, `/api/update`, `/api/timer`, `/api/power`, `/api/reset`.
- Reuse `api.rs` payload parsing and `controller.rs` update logic.
- Add CORS headers and static file hosting for frontend assets.

### 6) Time + scheduling
- Wire SNTP sync and RTC epoch tracking.
- Apply GMT offset and DST exactly like C++ behavior.
- Keep timer start logic + schedule-based OLED sleep.

### 7) OLED UI
- Port OLED drawing functions (static/dynamic UI, notifications).
- Ensure screen sleep logic matches the scheduler and API.

### 8) Home Assistant MQTT (optional feature)
- Add MQTT client and entity topics under a feature flag.
- Map HA selector indices/values exactly to current behavior.

### 9) Validation + tests
- Host-side unit tests already in place for logic.
- Add integration tests with mock hardware where possible.
- Run on-device smoke test for each feature (Wi-Fi, API, motor, OLED).

## Risks / notes
- Binary size and RAM are higher with ESP-IDF.
- Requires native ESP-IDF toolchain (slower builds).

## Deliverables
- Full Rust ESP32 firmware with feature parity.
- Documentation for build + flashing.
- On-device validation checklist.
