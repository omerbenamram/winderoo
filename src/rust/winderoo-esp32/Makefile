# Winderoo ESP32 Firmware - Build & Flash Makefile
#
# Usage:
#   make build          - Build firmware (release, with OLED)
#   make flash          - Flash to ESP32
#   make monitor        - Monitor serial output (resets device)
#   make tail           - Tail serial output (no reset)
#   make flash-monitor  - Build, flash, and monitor
#   make scan-wifi      - Scan WiFi from Mac (for signal comparison)
#   make erase-flash    - Erase entire flash (clears saved WiFi credentials)
#   make clean          - Clean build artifacts
#
# Configuration:
#   PORT        - Serial port (default: /dev/cu.usbserial-0001)
#   FEATURES    - Cargo features (default: oled)
#   BAUD        - Flash baud rate (default: 460800)
#   LOG_LEVEL   - Log level: trace/debug/info/warn/error (default: info)

# === Configuration ===
PORT       ?= /dev/cu.usbserial-0001
FEATURES   ?= oled
BAUD       ?= 460800
LOG_LEVEL  ?= info
MONITOR_DURATION ?= 30

# === ESP Toolchain Setup ===
# These environment variables are needed for the ESP32 Rust toolchain
ESP_PATH   := $(HOME)/.cargo/bin:$(HOME)/.rustup/toolchains/esp/xtensa-esp-elf/esp-15.2.0_20250920/xtensa-esp-elf/bin
ESP_CLANG  := $(HOME)/.rustup/toolchains/esp/xtensa-esp32-elf-clang/esp-20.1.1_20250829/esp-clang/lib

# Environment prefix for cargo commands
ENV := PATH="$(ESP_PATH):$(PATH)" LIBCLANG_PATH="$(ESP_CLANG)" RUSTUP_TOOLCHAIN=esp WINDEROO_LOG=$(LOG_LEVEL)

# === Paths ===
TARGET     := xtensa-esp32-none-elf
PROFILE    := release
BINARY     := target/$(TARGET)/$(PROFILE)/winderoo-esp32

# === Colors ===
GREEN  := \033[0;32m
YELLOW := \033[0;33m
CYAN   := \033[0;36m
RESET  := \033[0m

.PHONY: build flash monitor tail flash-monitor scan-wifi erase-flash clean help

help:
	@echo "$(CYAN)Winderoo ESP32 Firmware$(RESET)"
	@echo ""
	@echo "$(GREEN)Targets:$(RESET)"
	@echo "  build          Build firmware (release)"
	@echo "  flash          Flash to ESP32"
	@echo "  monitor        Monitor serial output (resets device)"
	@echo "  tail           Tail serial output (no reset)"
	@echo "  flash-monitor  Build, flash, and monitor"
	@echo "  scan-wifi      Scan WiFi networks from Mac"
	@echo "  erase-flash    Erase flash (clears WiFi credentials)"
	@echo "  clean          Clean build artifacts"
	@echo ""
	@echo "$(GREEN)Configuration:$(RESET)"
	@echo "  PORT=$(PORT)"
	@echo "  FEATURES=$(FEATURES)"
	@echo "  BAUD=$(BAUD)"
	@echo "  LOG_LEVEL=$(LOG_LEVEL)"
	@echo ""
	@echo "$(GREEN)Examples:$(RESET)"
	@echo "  make flash-monitor"
	@echo "  make flash-monitor LOG_LEVEL=debug"
	@echo "  make monitor MONITOR_DURATION=60"
	@echo "  make flash PORT=/dev/ttyUSB0"

build:
	@echo "$(CYAN)Building firmware (features: $(FEATURES))...$(RESET)"
	$(ENV) cargo build --release --features $(FEATURES)
	@echo "$(GREEN)Build complete: $(BINARY)$(RESET)"
	@ls -lh $(BINARY) | awk '{print "Size: " $$5}'

flash: build
	@echo "$(CYAN)Flashing to $(PORT)...$(RESET)"
	espflash flash --port $(PORT) --baud $(BAUD) $(BINARY)
	@echo "$(GREEN)Flash complete!$(RESET)"

monitor:
	@echo "$(CYAN)Monitoring $(PORT) for $(MONITOR_DURATION)s (resetting device)...$(RESET)"
	@echo "$(YELLOW)Press Ctrl+C to stop$(RESET)"
	@uv run --with pyserial python3 scripts/monitor.py $(PORT) $(MONITOR_DURATION)

tail:
	@echo "$(CYAN)Tailing $(PORT) for $(MONITOR_DURATION)s (no reset)...$(RESET)"
	@echo "$(YELLOW)Press Ctrl+C to stop$(RESET)"
	@uv run --with pyserial python3 scripts/monitor.py $(PORT) $(MONITOR_DURATION) --no-reset

flash-monitor: flash
	@sleep 0.5
	@$(MAKE) monitor MONITOR_DURATION=30

scan-wifi:
	@echo "$(CYAN)Scanning WiFi networks from Mac...$(RESET)"
	@swift -e '\
import CoreWLAN; \
let iface = CWWiFiClient.shared().interface()!; \
print("SSID                          | Signal  | Channel"); \
print("------------------------------|---------|--------"); \
for n in try! iface.scanForNetworks(withSSID: nil).sorted(by: { $$0.rssiValue > $$1.rssiValue }) { \
    let ssid = (n.ssid ?? "?").padding(toLength: 29, withPad: " ", startingAt: 0); \
    print("\(ssid) | \(n.rssiValue)dBm | ch\(n.wlanChannel?.channelNumber ?? 0)") \
}'

erase-flash:
	@echo "$(YELLOW)Erasing flash on $(PORT)...$(RESET)"
	@echo "$(YELLOW)This will clear all saved WiFi credentials!$(RESET)"
	espflash erase-flash --port $(PORT)
	@echo "$(GREEN)Flash erased. Device will enter provisioning mode on next boot.$(RESET)"

clean:
	@echo "$(CYAN)Cleaning build artifacts...$(RESET)"
	$(ENV) cargo clean
	@echo "$(GREEN)Clean complete$(RESET)"

# === Debug targets ===
.PHONY: build-debug flash-debug

build-debug:
	@echo "$(CYAN)Building firmware (debug, features: $(FEATURES))...$(RESET)"
	$(ENV) cargo build --features $(FEATURES)

flash-debug:
	@echo "$(CYAN)Building and flashing debug build...$(RESET)"
	$(ENV) cargo build --features $(FEATURES)
	espflash flash --port $(PORT) --baud $(BAUD) target/$(TARGET)/debug/winderoo-esp32
