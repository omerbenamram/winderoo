# Winderoo Embassy HAL

This crate provides a pure Rust, `embedded-hal`-based adapter layer for the
Winderoo firmware logic. It intentionally keeps hardware IO separate from the
state machine so the winding behavior can be tested on a host machine.

## What it contains
- `hardware.rs`: drivers + traits to map `ControllerEvent` values to GPIO/PWM/
  display calls.
- `runtime.rs`: a small wrapper that runs controller ticks and dispatches events.
- `tasks.rs`: async firmware loop helpers for embassy runtimes.
- No ESP-IDF dependencies (pure Rust stack).

## How it fits
- The core logic lives in `../osww-firmware` and is fully unit-tested.
- This crate is a HAL adapter. You supply concrete GPIO/PWM/display drivers and
  call `EventDispatcher::handle_event` for each event produced by the controller.

## Next steps (device integration)
- Provide an esp-hal/embassy runtime that:
  - initializes peripherals
  - feeds `Controller::tick` with RTC + time-of-day values
  - executes emitted `ControllerEvent`s with `EventDispatcher`
  - handles Wi-Fi, HTTP API, filesystem, and MQTT as separate tasks

> Note: version pins in `Cargo.toml` may need to align with your chosen ESP32
> toolchain and embassy/esp-hal release.
