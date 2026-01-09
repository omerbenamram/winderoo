# ESP32 Blinky - Build & Flash Makefile
#
# Usage:
#   make build          - Build firmware (release)
#   make flash          - Flash to ESP32
#   make flash-monitor  - Build, flash, and monitor
#   make monitor        - Monitor serial output
#   make clean          - Clean build artifacts
#
# Configuration:
#   PORT        - Serial port (default: /dev/cu.usbserial-0001)
#   BAUD        - Flash baud rate (default: 460800)

# === Configuration ===
PORT       ?= /dev/cu.usbserial-0001
BAUD       ?= 460800

# === ESP Toolchain Setup ===
ESP_PATH   := $(HOME)/.cargo/bin:$(HOME)/.rustup/toolchains/esp/xtensa-esp-elf/esp-15.2.0_20250920/xtensa-esp-elf/bin
ESP_CLANG  := $(HOME)/.rustup/toolchains/esp/xtensa-esp32-elf-clang/esp-20.1.1_20250829/esp-clang/lib

# Environment prefix for cargo commands
ENV := PATH="$(ESP_PATH):$(PATH)" LIBCLANG_PATH="$(ESP_CLANG)" RUSTUP_TOOLCHAIN=esp

# === Paths ===
TARGET     := xtensa-esp32-none-elf
PROFILE    := release
BINARY     := target/$(TARGET)/$(PROFILE)/blinky

# === Colors ===
GREEN  := \033[0;32m
YELLOW := \033[0;33m
CYAN   := \033[0;36m
RESET  := \033[0m

.PHONY: build flash monitor flash-monitor clean help

help:
	@echo "$(CYAN)ESP32 Blinky$(RESET)"
	@echo ""
	@echo "$(GREEN)Targets:$(RESET)"
	@echo "  build          Build firmware (release)"
	@echo "  flash          Flash to ESP32"
	@echo "  monitor        Monitor serial output"
	@echo "  flash-monitor  Build, flash, and monitor"
	@echo "  clean          Clean build artifacts"
	@echo ""
	@echo "$(GREEN)Configuration:$(RESET)"
	@echo "  PORT=$(PORT)"
	@echo "  BAUD=$(BAUD)"
	@echo ""
	@echo "$(GREEN)Examples:$(RESET)"
	@echo "  make flash-monitor"
	@echo "  make flash PORT=/dev/ttyUSB0"

build:
	@echo "$(CYAN)Building blinky...$(RESET)"
	$(ENV) cargo build --release
	@echo "$(GREEN)Build complete: $(BINARY)$(RESET)"
	@ls -lh $(BINARY) | awk '{print "Size: " $$5}'

flash: build
	@echo "$(CYAN)Flashing to $(PORT)...$(RESET)"
	espflash flash --port $(PORT) --baud $(BAUD) $(BINARY)
	@echo "$(GREEN)Flash complete!$(RESET)"

monitor:
	@echo "$(CYAN)Monitoring $(PORT)...$(RESET)"
	@echo "$(YELLOW)Press Ctrl+C to stop$(RESET)"
	espflash monitor --port $(PORT)

flash-monitor: flash
	@sleep 0.5
	@$(MAKE) monitor

clean:
	@echo "$(CYAN)Cleaning build artifacts...$(RESET)"
	$(ENV) cargo clean
	@echo "$(GREEN)Clean complete$(RESET)"
