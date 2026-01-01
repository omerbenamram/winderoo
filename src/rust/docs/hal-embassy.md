# Pure Rust HAL Architecture

This document explains the separation between the **core winding logic** and
**hardware IO** for the pure Rust (esp-hal + embassy) port.

## Layers

### 1) Core Logic (`osww-firmware`)
- `controller.rs` implements the winding state machine.
- `model.rs` contains typed state + settings snapshots.
- All logic is host-testable and `no_std` compatible (with `alloc`).

### 2) HAL Adapter (`osww-embassy`)
- `hardware.rs` defines small traits for GPIO/PWM/display control:
  - `MotorControl`
  - `LedControl`
  - `DisplayControl`
  - `SystemHooks`
- `EventDispatcher` maps `ControllerEvent` values to hardware actions.
- `runtime.rs` provides a thin wrapper that runs controller ticks and dispatches events.

### 3) Device Runtime (to be wired)
- An embassy task loop will:
  1. Poll the RTC / NTP time
  2. Call `Controller::tick` with the current epoch + time-of-day
  3. Dispatch all emitted `ControllerEvent`s
  4. Serve HTTP + Wi-Fi provisioning + MQTT in parallel tasks

## Why this separation
- Logic can be tested thoroughly without hardware.
- IO drivers can evolve independently (e.g., different OLEDs or motor drivers).
- The controller owns state; IO is a thin adapter layer.

## Where to look
- Core logic: `src/rust/osww-firmware/src/controller.rs`
- HAL adapter: `src/rust/osww-embassy/src/hardware.rs`
- Winding behavior: `src/rust/docs/winding-behavior.md`
