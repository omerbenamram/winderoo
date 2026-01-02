# Winderoo Rust + Embassy Guidelines

These notes are for contributors working on the Rust port and Embassy-based
runtime. They summarize the local Embassy book references in `external/embassy`
and the expectations for dependency versions and task structure.

## Embassy references (local source of truth)
- `external/embassy/docs/pages/runtime.adoc` – executor model, task rules.
- `external/embassy/docs/pages/best_practices.adoc` – buffer passing, stack usage.
- `external/embassy/docs/pages/time_keeping.adoc` – timers, tick rates.
- `external/embassy/docs/pages/project_structure.adoc` – recommended layout.

## Version policy (crates.io)
Use the latest published releases on crates.io unless a specific version is
required by an upstream dependency. At time of writing, the core stack is:
- `embassy-executor` 0.9.1
- `embassy-time` 0.5.0
- `embassy-sync` 0.7.2
- `embassy-net` 0.7.1
- `embassy-futures` 0.1.1
- `esp-hal` 1.0.0
- `esp-rtos` 0.2.0 (runtime + embassy executor for ESP32)
- `esp-radio` 0.17.0 (replacement for `esp-wifi`)
- `esp-alloc` 0.9.0
- `esp-println` 0.16.1
- `esp-backtrace` 0.18.1
- `picoserve` 0.17.1
- `edge-mdns` 0.7.0
- `edge-nal-embassy` 0.8.0
- `esp-hal-dhcp-server` 0.2.7
- `esp-storage` 0.8.1
- `littlefs2` 0.6.1
- `embedded-io-async` 0.7.0 (note: picoserve currently depends on 0.6.x)
- `heapless` 0.9.x

If a crate migrates (e.g., `esp-wifi` → `esp-radio`), update code paths and
features accordingly. If a crate is deprecated (e.g., `esp-hal-embassy`),
prefer the replacement (`esp-rtos` + embassy executor).

## Async task structure (Embassy “blessed” pattern)
- Use `#[esp_rtos::main]` + `#[embassy_executor::task]` for ESP32 runtime tasks.
- Keep long-running logic inside async tasks; avoid blocking calls inside tasks.
- Use `embassy_time::Timer`/`Ticker` for delays.
- Share state through `embassy_sync` primitives (channels/mutexes/signals).

## Buffer + memory best practices
- Avoid passing large buffers by value (see `best_practices.adoc`).
- Prefer stack buffers allocated once per task and reused.
- Use `StaticCell` for buffers that must be `'static` across tasks.

## HTTP / networking
- Prefer `embassy-net` for TCP/UDP.
- Use `picoserve` for HTTP routing on top of `embedded-io-async`.
- For embedded server tasks, prefer `picoserve::Server::listen_and_serve` with
  `embassy-net` TCP sockets.

## ESP32 Wi‑Fi
- Use `esp-radio` `WifiController` + `ModeConfig` for STA/AP provisioning.
- Keep provisioning state machine separate from controller logic.
- Read RSSI via `WifiController::rssi` and expose it via `StatusSnapshot`.
 - Initialize scheduler (`esp_rtos::start`) before calling `esp_radio::init`.

## Core logic purity
- `winderoo-firmware` must remain `no_std + alloc` and fully unit-testable.
- Keep controller state machine pure; IO lives in `osww-embassy`.
- All structs and modules must remain fully documented.
