# Winderoo ESP32 Firmware

## Quick Start (Makefile)

```bash
cd src/rust/winderoo-esp32

make help           # Show all targets and options
make flash-monitor  # Build, flash, and monitor serial output
make scan-wifi      # Compare WiFi signals (Mac vs ESP32)
make erase-flash    # Clear saved WiFi credentials
```

**Common options:**
```bash
make flash-monitor LOG_LEVEL=debug     # Enable debug logging
make monitor MONITOR_DURATION=60       # Monitor for 60 seconds
make flash PORT=/dev/ttyUSB0           # Use different serial port
```

---

## Troubleshooting Guide

Based on testing and the [impl Rust for ESP32 book](https://esp32.implrust.com/):

### Stack Overflow (`Detected a write to the stack guard value`)

ESP32 has limited SRAM (~320KB total). The stack shares memory with:
- **Heap** (`esp_alloc::heap_allocator!`) - keep ≤64KB for complex apps
- **Static buffers** - HTTP/TCP buffers, DNS buffers, etc.
- **Async task state** - Embassy tasks expand into state machines

**Fix**: Reduce heap size and buffer allocations:
```rust
// main.rs - heap should be ≤64KB
esp_alloc::heap_allocator!(size: 64 * 1024);

// HTTP task buffers - 1536 bytes is sufficient
let mut http_buffer = [0u8; 1536];
let mut tcp_rx_buffer = [0u8; 1536];
let mut tcp_tx_buffer = [0u8; 1536];
```

### Bootloader Mode (for flashing)

If `espflash` hangs at "Connecting...":
1. **Hold BOOT** button on ESP32
2. **Press RESET** while holding BOOT
3. **Release BOOT** after 1 second
4. Flash command should now proceed

### Linker Script

The firmware uses `linkall.x` from `esp-hal` (see book's [Linker Script chapter](https://esp32.implrust.com/std-to-no-std/linker-script.html)).
Configured in `.cargo/config.toml`:
```toml
rustflags = ["-C", "link-arg=-Tlinkall.x"]
```

### Embassy Timer Initialization

Embassy requires a timer before any async operations (see book's [Embassy chapter](https://esp32.implrust.com/embassy/blinky-with-embassy.html)):
```rust
let timg0 = TimerGroup::new(peripherals.TIMG0);
esp_rtos::start(timg0.timer0);  // Must be called early!
```

### WiFi Controller Order

Initialize WiFi controller **after** `esp_rtos::start()` but **before** spawning tasks.

---

## Manual Commands (without Makefile)

<details>
<summary>Click to expand environment setup and raw commands</summary>

### Environment Setup

```bash
export PATH="$HOME/.cargo/bin:$HOME/.rustup/toolchains/esp/xtensa-esp-elf/esp-15.2.0_20250920/xtensa-esp-elf/bin:$PATH"
export LIBCLANG_PATH="$HOME/.rustup/toolchains/esp/xtensa-esp32-elf-clang/esp-20.1.1_20250829/esp-clang/lib"
export RUSTUP_TOOLCHAIN=esp
```

### Build & Flash

```bash
cargo build --release --features oled
espflash flash --port /dev/cu.usbserial-0001 target/xtensa-esp32-none-elf/release/winderoo-esp32
```

### Monitor Serial

```bash
# Using Python (recommended - handles reset)
uv run --with pyserial python3 -c "
import serial, time, sys
port = serial.Serial('/dev/cu.usbserial-0001', 115200, timeout=0.1)
port.dtr = False; port.rts = True; time.sleep(0.1); port.rts = False
while True:
    data = port.read(4096)
    if data: sys.stdout.write(data.decode('utf-8', errors='replace')); sys.stdout.flush()
"

# Or direct serial (no reset)
stty -f /dev/cu.usbserial-0001 115200 && cat /dev/cu.usbserial-0001
```

</details>

---

## WiFi Provisioning Test Prompt

Copy this to start a new debug session:

```
I'm testing WiFi provisioning on my Winderoo ESP32 Rust firmware.

## Current state
- Firmware flashed successfully to ESP32 via `/dev/cu.usbserial-0001`
- Features enabled: `oled`
- The firmware implements a WiFiManager-style captive portal:
  - AP SSID: "Winderoo Setup"
  - Captive portal with DHCP DNS hint + wildcard DNS
  - HTML portal at root (`/`) serves `wifi.html`
  - Probe paths: `/generate_204`, `/gen_204`, `/hotspot-detect.html`, `/fwlink`, `/connecttest.txt`

## What I need to test
1. Connect phone/laptop to "Winderoo Setup" AP
2. Verify captive portal auto-opens (or navigate to 192.168.4.1)
3. Enter WiFi credentials via the portal
4. Verify device connects to home WiFi and reboots
5. Access http://winderoo.local/ after provisioning

## Serial monitor command
```bash
espflash monitor --port /dev/cu.usbserial-0001
```

## Key files
- `src/rust/winderoo-esp32/src/main.rs` - main firmware entry
- `src/rust/osww-embassy/src/wifi.rs` - WiFi manager
- `src/rust/osww-embassy/src/http.rs` - HTTP routes including captive portal
- `src/rust/osww-embassy/src/captive_portal.rs` - DNS wildcard responder
- `data/wifi.html` - portal UI

Help me debug the serial output and verify the WiFi provisioning flow works correctly.
```

---

## Testing WiFi Provisioning

### Expected Boot Sequence

```
INFO - winderoo-esp32 booting
INFO - http-ap: Listening on TCP:80...
INFO - http-sta: Listening on TCP:80...
```

### Manual Test Steps

1. **Check for "Winderoo Setup" AP** on your phone/laptop WiFi list
2. **Connect to it** (no password by default)
3. **Captive portal should auto-open** - if not, navigate to `http://192.168.4.1/`
4. **Enter your home WiFi credentials**
5. **Device reboots** and connects to your WiFi
6. **Access** `http://winderoo.local/` (or check router for assigned IP)

### Serial Monitor

```bash
make monitor                    # Resets device and monitors for 30s
make monitor MONITOR_DURATION=60  # Monitor for 60 seconds
```

### DNS/Captive Portal Endpoints

The AP HTTP server responds to these probe paths (triggers captive portal UI):
- `/generate_204` - Android
- `/gen_204` - Android (alternate)
- `/hotspot-detect.html` - iOS/macOS
- `/fwlink` - Windows
- `/connecttest.txt` - Windows (alternate)

### Memory Budget (ESP32 SRAM)

| Component | Size | Notes |
|-----------|------|-------|
| Heap | 64KB | For JSON parsing, alloc |
| HTTP buffers (×2 servers) | ~9KB | 1536×3×2 |
| DNS buffer | ~2.5KB | 512×5 |
| OLED frame buffer | ~1KB | 128×64÷8 |
| Stack | ~50KB+ | Shared with async state |
| **Total available** | ~190KB | After ROM/bootloader |
