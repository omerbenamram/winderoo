# winderoo-esp32

ESP32 runtime crate wiring `winderoo-firmware` + `winderoo-embassy` into a runnable async
application using `esp-rtos`, `esp-radio`, `embassy-net`, and `picoserve`.

## What’s implemented

- **Wi‑Fi provisioning flow**
  - Try saved credentials first
  - Fall back to SoftAP provisioning (`SSID: "Winderoo Setup"`, open network)
  - `/api/wifi` writes credentials and triggers reconnect
- **HTTP API + static assets**
  - `/api/status`, `/api/timer`, `/api/power`, `/api/update`, `/api/reset`, `/api/wifi`
  - Static web assets served from `data/assets/*` (embedded via `include_bytes!`)
  - Server runs on **both** STA + AP stacks (port 80)
- **DHCP server (AP mode)** via `esp-hal-dhcp-server`
- **SNTP sync** via `winderoo-embassy::sntp::UdpSntpClient` (best-effort)
- **mDNS** (optional; enabled by default)
  - Advertises `winderoo.local` and `_winderoo._tcp` on port 80
- **Persistence**
  - Settings and Wi‑Fi credentials stored as length-prefixed JSON blobs in flash
  - See “Flash layout” below
- **Motor + LED**
  - Motor H-bridge on two GPIO pins
  - Simple GPIO blink patterns for LED (matches `LedPattern`)
- **OLED (optional)** via `ssd1306`
  - Basic status UI + notifications (128x64 @ I2C addr 0x3C)
- **Home Assistant (optional)** via `embassy-ha`
  - MQTT discovery + command/control using a compact set of entities (bounded by `embassy-ha`’s entity limit)

## Pin defaults

These mirror the Arduino defaults in `src/platformio/osww-server/src/main.cpp`:

- **motor IN1**: GPIO25
- **motor IN2**: GPIO26
- **status LED**: GPIO0 (change if your board uses GPIO2)

## Flash layout

This runtime reserves **64 KiB** at the end of flash:

- **settings**: 32 KiB
- **Wi‑Fi credentials**: 32 KiB

If your partition table already uses the end-of-flash for LittleFS/SPIFFS, you must adjust it
to avoid overlap.

## Building / flashing

This crate targets `xtensa-esp32-none-elf`. Use the ESP Rust toolchain (via `espup`) and then
build/flash with your preferred workflow (e.g. `cargo-espflash`).

## Features

- **`mdns`**: enable mDNS responder (default)
- **`oled`**: enable SSD1306 OLED support
- **`home-assistant`**: enable Home Assistant MQTT discovery/control

### Home Assistant configuration

Set `WINDEROO_HA_BROKER` at build time (e.g. `"192.168.1.10:1883"`). If left unset, the firmware
will boot with HA disabled (matching the Arduino “placeholder config” behavior).

