# I2C Display Test (ESP32 + SSD1306)

Minimal test firmware to validate wiring + orientation for an I2C OLED display (SSD1306 128x64),
using `esp-hal` + Embassy timing.

## Wiring (default pins)

| OLED | ESP32 |
|------|------|
| VCC  | 3V3  |
| GND  | GND  |
| SDA  | GPIO21 |
| SCL  | GPIO22 |

Most SSD1306 modules use I2C address **0x3C** (some are **0x3D**).

## What you should see

- A border rectangle + crosshair (helps verify rotation/origin)
- `I2C OLED TEST` header
- An increasing `#00001` counter
- A moving filled 8x8 square along the bottom

## Prerequisites

Install the ESP Rust toolchain using [espup](https://github.com/esp-rs/espup) and source the env:

```bash
cargo install espup
espup install
source ~/export-esp.sh
```

## Build

```bash
cargo build --release
```

Binary output:
`target/xtensa-esp32-none-elf/release/i2c-display-test`

## Flash + monitor

```bash
cargo install espflash
espflash flash --monitor target/xtensa-esp32-none-elf/release/i2c-display-test
```

## Tweaks

Edit `src/main.rs`:

- `OLED_ROTATE_180`: flip the display
- `OLED_INVERT`: invert pixels
- I2C pins: change `.with_sda(peripherals.GPIO21)` / `.with_scl(peripherals.GPIO22)`

## Troubleshooting quick hits

- If it stays blank: confirm **VCC/GND**, and that the module is okay with **3.3V**
- If you see I2C errors: shorten wires, add pullups (many modules have them), lower I2C speed
- Some modules expose **RES/RESET**: ensure it’s tied high (or drive it with a GPIO)
