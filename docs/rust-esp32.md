> [Back to main page](../README.md)

# Rust ESP32 Firmware (ESP-IDF)

This repo now includes a Rust port of the ESP32 firmware built on **ESP-IDF** (not Embassy). It mirrors the C++ feature set: Wi-Fi provisioning, HTTP API, mDNS, SNTP time sync, LittleFS static UI hosting, optional OLED, optional PWM motor control, and optional Home Assistant MQTT.

## Hardware Target
- ESP32 DevKit v1 (ESP32-WROOM-32)
- Default pin map used by the Rust firmware:
  - LED: GPIO2 (on-board LED)
  - Motor A/B: GPIO25 / GPIO26
  - Button: GPIO13 (pulldown)
  - OLED (I2C): SDA GPIO21, SCL GPIO22, address 0x3C

## Prerequisites
- Rust toolchain via `espup` (recommended by esp-rs)
  - This repo pins the project toolchain to `esp` via `rust-toolchain.toml` (required for Xtensa / ESP32).
  - For best IDE navigation, ensure the `rust-src` component is installed for the `esp` toolchain.
- Flashing tool (`espflash`/`cargo-espflash` or `esptool.py`)
- LittleFS image builder (`mklittlefs` or `littlefs-python`)

## Build & Flash (Firmware)
From `src/rust/osww-firmware`:

```bash
cargo esp32-build
```

Equivalent explicit command:

```bash
cargo build -Z build-std=std,panic_abort -Z build-std-features=panic_immediate_abort --release --target xtensa-esp32-espidf -F esp32
```

Optional features (match the C++ flags):
- `oled` (SSD1306 I2C display)
- `oled-invert` (invert OLED pixels; matches C++ `OLED_INVERT_SCREEN`)
- `oled-rotate-180` (rotate OLED 180°; matches C++ `OLED_ROTATE_SCREEN_180`)
- `pwm-motor` (LEDC PWM motor control)
- `home-assistant` (MQTT discovery + control)

Example:

```bash
cargo build -Z build-std=std,panic_abort -Z build-std-features=panic_immediate_abort --release --target xtensa-esp32-espidf -F "esp32 oled pwm-motor home-assistant"
```

Then flash the firmware using your preferred tool. Example with `cargo-espflash`:

```bash
cargo espflash --release --monitor
```

Or use the repo-provided runner (flash + monitor):

```bash
cargo esp32
```

## Build & Flash (LittleFS UI)
The UI assets live in `data/` at the repo root. Build a LittleFS image and flash it to the `littlefs` partition defined in `src/rust/osww-firmware/partitions.csv`:

Partition details:
- Offset: `0x290000`
- Size: `0x170000`
- Page size: 256 bytes
- Block size: 4096 bytes

Example with `mklittlefs` + `esptool.py` (run from the repo root):

```bash
mklittlefs -c data -p 256 -b 4096 -s 0x170000 littlefs.bin
esptool.py --chip esp32 write_flash 0x290000 littlefs.bin
```

If you change anything under `data/`, rebuild and re-flash the LittleFS image.

## Home Assistant (MQTT)
When `home-assistant` is enabled, the firmware reads the MQTT broker settings from environment variables at build time (or runtime on host builds):

- `HOME_ASSISTANT_BROKER` (required, e.g. `192.168.1.10:1883`)
- `HOME_ASSISTANT_USERNAME` (optional)
- `HOME_ASSISTANT_PASSWORD` (optional)

Example:

```bash
HOME_ASSISTANT_BROKER=192.168.1.10:1883 cargo build -Z build-std=std,panic_abort -Z build-std-features=panic_immediate_abort --release --target xtensa-esp32-espidf -F "esp32 home-assistant"
```

## Runtime Behavior (Quick Notes)
- On first boot (or after reset), the ESP32 creates a Wi-Fi AP named **"Winderoo Setup"**.
- Connect, enter Wi-Fi credentials, and the device will reboot and join your network.
- If the portal does not open automatically, browse to `http://192.168.4.1/`.
- The UI is served from `http://winderoo.local/` (mDNS).

## Troubleshooting
- If the OLED is blank, verify SDA/SCL wiring and the I2C address (default 0x3C).
- If the UI 404s, re-flash the LittleFS image.
- For PWM motor control, verify you're using a PWM-capable driver (e.g., MX1508).
