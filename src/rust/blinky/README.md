# ESP32 Blinky

Minimal green LED blinky using `esp-hal` + Embassy async runtime.

## Wiring Diagram

```
                    ESP32 DevKit
                   ┌───────────┐
                   │           │
              VIN  │●         ●│  3V3
              GND  │●         ●│  GND
              D13  │●         ●│  D15
              D12  │●         ●│  D2  ←──┐
              D14  │●         ●│  D4     │
              D27  │●         ●│  RX2    │
              D26  │●         ●│  TX2    │
              D25  │●         ●│  D5     │
              D33  │●         ●│  D18    │
              D32  │●         ●│  D19    │
              D35  │●         ●│  D21    │
              D34  │●         ●│  RX0    │
               VN  │●         ●│  TX0    │
               VP  │●         ●│  D22    │
                   │  [USB-C]  │  D23    │
                   │ [EN][BOOT]│         │
                   └───────────┘         │
                                         │
    ┌────────────────────────────────────┘
    │
    │    RESISTOR (220-330Ω)        LED (Green)
    │   ┌─────────────────┐      ┌─────────┐
    └───┤▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓├──────┤ ◄──┃   ├───┐
        └─────────────────┘      │  (+)(-) │   │
                                 └─────────┘   │
                                    Anode  Cathode
                                   (long)  (short)
                                               │
                                               │
    ┌──────────────────────────────────────────┘
    │
    └─── to GND pin on ESP32
```

### Breadboard Layout

**Board**: ESP32 DOIT DevKit (30-pin), rows 16-30 on breadboard.

```
    POWER RAILS          MAIN BREADBOARD HOLES              POWER RAILS
    (─) (+)      a   b   c   d   e  │  f   g   h   i   j    (─) (+)
     │   │       ─────────────────────────────────────       │   │
     ●   ○   1   ○   ○   ○   ○   ○  │  ○   ○   ○   ○   ○     ○   ●
     :   :       :                  │                  :     :   :
     ●   ○  15   ○   ○   ○   ○   ○  │  ○   ○   ○   ○   ○     ○   ●  ← free row
     ─────────────────────────────────────────────────────────────────
     ●   ○  16   ■  EN  ───■───■───■──│──■───■───■───  D23 ■     ○   ●  ← ESP32 START
     ●   ○  17   ■  VP              │              D22 ■     ○   ●
     ●   ○  18   ■  VN              │              TX0 ■     ○   ●
     ●   ○  19   ■  D34             │              RX0 ■     ○   ●
     ●   ○  20   ■  D35             │              D21 ■     ○   ●
     ●   ○  21   ■  D32             │              D19 ■     ○   ●
     ●   ○  22   ■  D33             │              D18 ■     ○   ●
     ●   ○  23   ■  D25             │              D5  ■     ○   ●
     ●   ○  24   ■  D26             │              TX2 ■     ○   ●
     ●   ○  25   ■  D27             │              RX2 ■     ○   ●
     ●   ○  26   ■  D14             │              D4  ■     ○   ●  ← D4 here! (wire to LED)
     ●   ○  27   ■  D12             │              D2  ■     ○   ●  ← D2 = onboard blue LED
     ●   ○  28   ■  D13             │              D15 ■     ○   ●
     ●   ○  29   ■  GND             │              GND ■     ○   ●  ← GND here!
     ●   ○  30   ■  VIN ───■───■───■──│──■───■───■───  3V3 ■     ○   ●  ← ESP32 END
     ─────────────────────────────────────────────────────────────────

    The (─) rail runs vertically - all holes in that column are connected!
    Same for (+) rail. Use these to distribute GND and 3.3V.
```

### Where to Put Your Components (rows 1-15 are FREE above the ESP32)

```
    (─) (+)      a   b   c   d   e  │  f   g   h   i   j    (─) (+)
     │   │       ─────────────────────────────────────       │   │
     ●   ○   8   ○   ○   ○   ○   ○  │  ○   ○   ○   ○   ○     ○   ●
     │
     │   RESISTOR: row 9, from c9 to g9
     │   LED LONG LEG: row 9, h9 (connects to resistor via shared f-g-h-i-j)
     │   LED SHORT LEG: row 10, h10 (wire to GND rail)
     │
     ●   ○   9   ○   ○  ●═══════════●  ◉   ○   ○     ○   ●
     │                  ↑  RESISTOR ↑  ↑
     │                (c9)        (g9)(h9)
     │                              └──┴── connected! (f-g-h-i-j share row 9)
     │                                 │
     │                                 │ LED long leg (+) at h9
     │                                 │
     ●   ○  10   ○   ○   ○   ○   ○  │  ○   ○  ◉   ○   ○     ○   ●
     │                                        │
     │                                        └── LED short leg (−) at h10
     │                                                    │
     │                                                    │ WIRE
     │                                                    ↓
     ●←━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━●
     │                        GND RAIL (─)
     │
     │   Also connect ESP32 GND (row 29, col j) to this rail!
```

### Your 3 Wires

| # | FROM | TO | Color |
|---|------|-----|-------|
| 1 | **D4** (row 26, col j) | **Resistor** (row 9, col b) | 🟡 Yellow |
| 2 | **LED short leg** (row 10, col h) | **(−) GND rail** on left | ⚫ Black |
| 3 | **ESP32 GND** (row 29, col j) | **(−) GND rail** on left | ⚫ Black |

### LED Orientation (Important!)

```
    GREEN LED (5mm)

         ┌───┐
         │   │  ← rounded top (lens)
         │ ● │  ← flat edge on CATHODE side
         └┬─┬┘
          │ │
          │ │
          │ └──── SHORT leg = CATHODE (-) → goes to GND
          │
          └────── LONG leg = ANODE (+) → goes to resistor
```

### Wire Connections Summary

| From | To | Wire Color (suggestion) |
|------|----|------------------------|
| D4 (row 26, col j) | Resistor (row 9, col b) | Yellow or Orange |
| LED cathode (row 10, col h) | GND rail | Black |
| ESP32 GND (row 29, col j) | GND rail | Black |

### Parts Needed

| Part | Value | Notes |
|------|-------|-------|
| Resistor | 220Ω - 330Ω | Limits current to ~10-15mA |
| LED | Green 5mm | Any standard LED works |
| Jumper wires | 2x | Connect D2→resistor, LED→GND |

### Quick Wiring Steps

1. **D2 → Resistor**: Jumper wire from D2 pin to one leg of resistor
2. **Resistor → LED anode**: Other resistor leg to LED **long leg** (+)
3. **LED cathode → GND**: LED **short leg** (-) to any GND pin

## Prerequisites

Install the ESP Rust toolchain using [espup](https://github.com/esp-rs/espup):

```bash
cargo install espup
espup install
```

## Building

Source the toolchain environment, then build:

```bash
# On macOS/Linux
source ~/export-esp.sh

# Build release
cargo build --release
```

The binary will be at `target/xtensa-esp32-none-elf/release/blinky`.

## Flashing

Use [espflash](https://github.com/esp-rs/espflash):

```bash
cargo install espflash

# Flash and monitor
espflash flash --monitor target/xtensa-esp32-none-elf/release/blinky
```

Or just monitor:

```bash
espflash monitor
```

## Configuration

Edit `src/main.rs` to change the LED GPIO pin:

```rust
/// GPIO pin for the green LED. Change this to match your board.
const LED_GPIO: u8 = 2;
```

Common GPIO options:
- **GPIO2**: Built-in LED on many ESP32 dev boards
- **GPIO0**: Winderoo status LED
- **GPIO5, GPIO18, GPIO19**: Common dev board choices

## Logging

Log level is set to `Info` by default. The output is sent over UART (115200 baud).

Example output:
```
[0] INFO - blinky v0.1.0 starting...
[1] INFO - initializing HAL
[2] INFO - HAL initialized (cpu_clock=max)
[3] INFO - starting RTOS scheduler + embassy time driver
[4] INFO - RTOS started
[5] INFO - configuring GPIO2 as LED output
[6] INFO - spawning blink task
[7] INFO - blink_task: starting LED blink loop on GPIO2
[8] INFO - main loop idle
[508] INFO - LED ON
[1008] INFO - LED OFF
[1508] INFO - LED ON
...
```
