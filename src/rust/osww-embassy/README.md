# Winderoo Embassy HAL

This crate provides a pure Rust, `embedded-hal`-based adapter layer for the
Winderoo firmware logic. It intentionally keeps hardware IO separate from the
state machine so the winding behavior can be tested on a host machine.

## What it contains
- `hardware.rs`: drivers + traits to map `ControllerEvent` values to GPIO/PWM/
  display calls.
- `tasks.rs`: async firmware loop helpers plus the full controller task.
- `state.rs`: shared status cache + system signals.
- `http.rs`: minimal HTTP parser + `/api/*` router.
- Optional future: swap to `picoserve` for a full embedded HTTP router.
- `wifi.rs`: Wi‑Fi provisioning state machine.
- `sntp.rs`: SNTP packet helpers and UDP client.
- `system.rs`: persistence, NTP sync, and reset coordination.
- `time.rs`: RTC adapters + time-of-day helpers.
- `esp32.rs`: `esp-radio` + `esp-hal` adapters (feature `esp32`).
- No ESP-IDF dependencies (pure Rust stack).

## How it fits
- The core logic lives in `../osww-firmware` and is fully unit-tested.
- This crate is a HAL adapter. You supply concrete GPIO/PWM/display drivers and
  call `EventDispatcher::handle_event` for each event produced by the controller.

## Next steps (device integration)
- Wire the embassy tasks together:
  - Wi‑Fi task updates `WifiStatus` + accepts provisioning updates
  - HTTP task maps `/api/*` endpoints to `RuntimeCommand`s
  - Controller task runs `Controller::tick` and dispatches events
  - System task persists settings + performs NTP sync

> Note: version pins in `Cargo.toml` may need to align with your chosen ESP32
> toolchain and embassy/esp-hal release.
