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
- `tasks.rs` defines:
  - `FirmwareTask` (simple loop)
  - `ControllerTask` (full async runtime with command channel + status cache)
- `state.rs` provides shared caches (`StatusCache`, `WifiStatus`, `SystemSignals`).
- `http.rs` implements the embedded HTTP API router (+ embassy-net server helper).
- HTTP is intentionally minimal; if we want a full router later, evaluate
  `picoserve` (no_std HTTP server built on `embedded-io-async`).
- `wifi.rs` hosts the Wi-Fi provisioning state machine.
- `sntp.rs` handles SNTP packet parsing + UDP client.
- `system.rs` persists settings, triggers NTP sync, and handles resets.
- `time.rs` wraps the RTC and exposes a `TimeSource` adapter.
- `esp32.rs` provides `esp-radio` + reset adapters when the `esp32` feature is enabled.

### 3) Device Runtime (embassy tasks)
An embassy-based runtime wires the tasks together as follows:

```
Wi-Fi Task  ─┐
HTTP Task   ├─> Runtime command channel ─┐
            └────────────────────────────┤
Controller Task (Controller + EventDispatcher)
            ├─ updates StatusCache
            └─ emits SystemSignals (persist/sync/reset)
System Task ─────────────────────────────┘
```

Task responsibilities:
1. **Wi‑Fi task**: connect using saved credentials, fall back to SoftAP, and
   expose RSSI via `WifiStatus`.
2. **HTTP task**: serve `/api/*` endpoints and translate requests into
   `RuntimeCommand` messages.
3. **Controller task**: runs the winding algorithm, dispatches IO events, and
   updates `StatusCache`.
4. **System task**: persists settings, performs NTP sync, and handles reset.

## Why this separation
- Logic can be tested thoroughly without hardware.
- IO drivers can evolve independently (e.g., different OLEDs or motor drivers).
- The controller owns state; IO is a thin adapter layer.

## Where to look
- Core logic: `src/rust/osww-firmware/src/controller.rs`
- HAL adapter: `src/rust/osww-embassy/src/hardware.rs`
- Runtime tasks: `src/rust/osww-embassy/src/tasks.rs`
- HTTP + Wi-Fi: `src/rust/osww-embassy/src/http.rs`, `src/rust/osww-embassy/src/wifi.rs`
- System + NTP: `src/rust/osww-embassy/src/system.rs`, `src/rust/osww-embassy/src/sntp.rs`
- Winding behavior: `src/rust/docs/winding-behavior.md`
