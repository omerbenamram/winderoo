# Rust Port Plan: esp-hal + Embassy (no_std async)

## Goal
Deliver a fully async, pure-Rust ESP32 runtime that preserves the C++ firmware
behavior while keeping the winding algorithm pure and host-testable.

## Current status (2026-01-01)
- Core logic (`winderoo-firmware`) is `no_std + alloc` with host tests and
  documented winding behavior.
- HAL adapter (`winderoo-embassy`) exists with controller tasks, Wi‑Fi manager
  state machine, SNTP client, system signals, and status cache.
- Minimal HTTP router exists but must be replaced with a crate (picoserve).
- No ESP32 entrypoint/runtime crate yet.

## Plan (updated)

### 1) HTTP API + static assets (crate-based)
- Replace the minimal router with `picoserve` (`embedded-io-async` + embassy).
- Keep API behavior aligned with C++ endpoints:
  - `/api/status`, `/api/timer`, `/api/power`, `/api/update`, `/api/reset`, `/api/wifi`
  - CORS + OPTIONS preflight handling.
- Serve static assets (gzip) from embedded bytes or FS with cache headers.
- Add host-side tests for request routing + payload parsing.

### 2) Storage + settings
- Implement flash-backed storage with `esp-storage`.
- Add LittleFS (`littlefs2`) driver backed by flash partition.
- Implement a `SettingsStore` that reads/writes `/settings.json` (C++ parity).
- Document partition layout + offsets for settings/FS.

### 3) ESP32 runtime crate (entrypoint)
- New crate (e.g., `winderoo-esp32`) with `#[esp_hal::main]` entrypoint.
- Start scheduler via `esp_rtos::start(...)` and run embassy executor manually.
- Initialize Wi‑Fi via `esp_radio::init` + `wifi::new`.
- Build both STA and AP stacks (embassy-net), run DHCP server for AP.
- Wire tasks:
  - network runner(s)
  - Wi‑Fi manager task
  - HTTP server tasks
  - controller task
  - system task (settings + NTP + reset)
  - mDNS responder
  - optional HA MQTT

### 4) Time + RTC
- Implement monotonic RTC adapter (epoch offset + embassy time).
- Implement DNS-based SNTP client and hook into system task.

### 5) mDNS
- Advertise `winderoo.local` + `_winderoo._tcp` using `edge-mdns` +
  `edge-nal-embassy` on the STA stack.

### 6) Optional Home Assistant MQTT
- Feature-gate HA integration.
- Provide entity mapping for power, status, timer, direction, RPD, etc.
- Use an embassy-compatible MQTT crate (to be selected during implementation).

### 7) Documentation + tests
- Build/flash/provisioning guide (ESP32, cargo-espflash).
- Runtime architecture diagram + task responsibilities.
- Expand tests if any winding behavior gaps are identified.

## Risks / notes
- `esp-hal-embassy` is deprecated; use `esp-rtos` + embassy executor.
- SoftAP provisioning requires DHCP server; use `esp-hal-dhcp-server`.
- Keep IO isolated to `winderoo-embassy`/runtime, preserve pure algorithm.

