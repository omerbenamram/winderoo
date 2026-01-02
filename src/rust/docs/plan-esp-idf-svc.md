# Rust Port Plan: ESP-IDF (esp-idf-svc)

## Goal
Port the existing ESP32 firmware to Rust using ESP-IDF services to preserve feature parity with the current Arduino/C++ implementation while minimizing rework.

## Why this plan
- Closest mapping to current functionality (Wi-Fi provisioning, HTTP server, mDNS, NTP/RTC, FS, MQTT).
- Most stable and documented path for ESP32 in Rust today.
- Fastest route to a working device with minimal behavior drift.

## Current parity status (vs `src/platformio/osww-server`) — 2026-01-02

### Baseline
- **PlatformIO (Arduino/C++)**: `src/platformio/osww-server/src/main.cpp` (+ `src/platformio/osww-server/src/utils/*`)
- **Rust (ESP-IDF)**: `src/rust/osww-firmware` (runtime in `src/rust/osww-firmware/src/esp32.rs`, portable logic in `api.rs`, `controller.rs`, `settings.rs`, `model.rs`, `time.rs`)

### Parity matrix

| Area | PlatformIO (Arduino/C++) | Rust (`esp-idf-svc`) | Notes / deltas |
|---|---|---|---|
| Build + SDK | PlatformIO/Arduino | Cargo + `esp-idf-sys`/`esp-idf-svc` (`esp32` feature) | Rust pulls LittleFS + mDNS via ESP component registry |
| Wi‑Fi provisioning | WiFiManager captive portal | AP-mode config portal + restart | Portal UX differs (no dark mode / timeout); functional parity |
| Wi‑Fi credential persistence | WiFiManager internal | NVS (`wifi/ssid`, `wifi/password`) | Equivalent outcome |
| Filesystem | Arduino LittleFS | `esp_idf_svc::fs::littlefs` mounted at `"/littlefs"` | C++ paths are `/settings.json` and `/css|/js|/index.html`; Rust expects the same assets under `"/littlefs"` |
| Settings schema | JSON settings file | `StoredSettings` JSON | Keys match C++ (`savedStatus`, `savedTPD`, `gmtOffset`, `dst`, screen schedule fields, etc.) |
| HTTP API routes | AsyncWebServer | `EspHttpServer` | `/api/status`, `/api/timer`, `/api/power`, `/api/update`, `/api/reset` implemented |
| CORS / OPTIONS | Global default headers + 404 handler for OPTIONS | Per-response CORS headers + wildcard OPTIONS handler | Equivalent behavior for browser clients |
| Static frontend hosting | `serveStatic()` from LittleFS | Wildcard static handler from LittleFS (+ `.gz` support) | Repo currently doesn’t include uploaded FS assets; still requires a build/upload step |
| mDNS | `MDNS.begin("winderoo"); addService("_winderoo","_tcp",80)` | `EspMdns::set_hostname("winderoo"); add_service("_winderoo","_tcp",80)` | Match |
| Time sync | NTPClient + ESP32Time RTC | SNTP (`EspSntp`) | **Confirmed semantic**: API “epoch” values are treated as *local-shifted* epochs (UTC + offset [+DST]). Frontend formats with timezone `UTC` to display local wall-clock time. Rust should emit shifted epochs for parity. |
| Timer start | RTC hour/minute comparison | `controller.tick()` uses `time_of_day_from_epoch()` | Match |
| Winding routine | Pause windows + BOTH mode direction toggling + random gate | Same algorithm in `controller.rs` | Match (including progress 0..1) |
| Motor control | GPIO 25/26, optional PWM motor driver | GPIO 25/26, optional LEDC PWM (`pwm-motor`) | Same behavior, different driver backend |
| LED patterns | PWM pulse + slow/fast blink | Same timings via LEDC | Match |
| OLED UI | Adafruit_SSD1306 layout | `ssd1306` + `embedded-graphics` layout | Core UI matches; missing C++ config toggles (invert/rotate) + a few cosmetic screens/icons |
| Screen scheduling | Same-day + overnight schedule | `ScreenSchedule::should_be_awake()` + controller enforcement | Match |
| Home Assistant MQTT | ArduinoHA entities | MQTT + HA discovery (`home-assistant` feature) | Implemented but not guaranteed byte-for-byte identical entity metadata |
| Reset behavior | `/api/reset` resets WiFiManager + reboot | `/api/reset` clears NVS Wi‑Fi creds + reboot | Equivalent outcome |

### Time/epoch semantics (details)

**Why this matters**: the UI assumes `currentTimeEpoch` is already “local time”, not a pure UTC epoch.

- **C++ behavior**:
  - Applies `setTimeOffset((gmtOffset [+DST]) * 3600)` to `NTPClient` *before* reading epoch.
  - Sets `ESP32Time rtc` from those already-offset components.
  - `/api/status` returns `rtc.getEpoch()` and routine epochs derived from it ⇒ epochs are effectively **UTC + offset (+DST)**.

- **Frontend proof point (Angular)**:
  - The UI renders the device clock using `date:'mediumTime':'UTC'`. That only displays the configured local wall-clock time if the epoch is already shifted.
  - Progress is computed using the three epoch fields; they must all be in the same epoch domain.

- **Rust parity implementation**:
  - Internals keep system time in UTC (SNTP) and compute time-of-day via `time_of_day_from_epoch(epoch_utc, gmtOffset, dst)`.
  - API boundary shifts epochs via `epoch_with_offset(epoch_utc, gmtOffset, dst)` for:
    - `currentTimeEpoch`
    - `startTimeEpoch`
    - `estimatedRoutineFinishEpoch`
  - Home Assistant `rtc_epoch` is published using the same shifted epoch.

### Phase status (today)
- [x] Project skeleton + build (ESP-IDF + `esp-idf-svc`)
- [x] Hardware layer (motor + LED + button + optional OLED)
- [x] Wi‑Fi provisioning / portal
- [x] Filesystem + settings JSON
- [x] HTTP API + static hosting + CORS
- [x] Time sync + scheduling
- [x] OLED UI (core parity; some polish deltas)
- [x] Home Assistant MQTT (feature-gated)
- [~] Validation: unit tests exist for host-side logic; on-device smoke checklist still needed

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
