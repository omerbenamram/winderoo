//! ESP32 I2C display test (SSD1306 128x64).
//!
//! Default wiring (typical ESP32 DevKit):
//! - VCC -> 3V3
//! - GND -> GND
//! - SDA -> GPIO21
//! - SCL -> GPIO22
//!
//! If your OLED is blank:
//! - Double-check VCC/GND orientation (some modules are 5V tolerant, some are not).
//! - Many SSD1306 modules are at I2C address 0x3C, some are 0x3D.
//! - Some modules require RESET to be tied high (or driven by a GPIO).

#![no_std]
#![no_main]

// Embed ESP-IDF app descriptor for bootloader compatibility.
esp_bootloader_esp_idf::esp_app_desc!();

// Keep panic + exception handlers linked in.
use esp_backtrace as _;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::Config as HalConfig;
use esp_println::logger::init_logger;
use log::{info, warn};

use embedded_graphics::mono_font::{
    ascii::{FONT_10X20, FONT_6X10},
    MonoTextStyle,
};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Baseline, Text};

const I2C_FREQ_KHZ: u32 = 400;
const OLED_INVERT: bool = false;
const OLED_ROTATE_180: bool = false;
const FRAME_MS: u64 = 250;

/// Timestamp provider for esp-println logger (milliseconds since boot).
#[no_mangle]
pub extern "Rust" fn _esp_println_timestamp() -> u64 {
    esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis()
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    init_logger(log::LevelFilter::Info);
    info!(
        "i2c-display-test v{} starting...",
        env!("CARGO_PKG_VERSION")
    );

    // Initialize HAL with a fast clock (keeps I2C + logging snappy).
    let config = HalConfig::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Start RTOS scheduler + embassy time driver (TIMG0.timer0).
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    info!(
        "configuring I2C0 @{}kHz (SDA=GPIO21, SCL=GPIO22)",
        I2C_FREQ_KHZ
    );
    let i2c_config = I2cConfig::default().with_frequency(Rate::from_khz(I2C_FREQ_KHZ));
    let i2c = I2c::new(peripherals.I2C0, i2c_config)
        .expect("I2C config error")
        .with_sda(peripherals.GPIO21)
        .with_scl(peripherals.GPIO22);

    // SSD1306 (I2C) display init.
    // NOTE: `I2CDisplayInterface::new(...)` uses the SSD1306 default address (usually 0x3C).
    // If your module is 0x3D, adjust the interface creation accordingly (see ssd1306 docs).
    use ssd1306::prelude::*;
    let interface = ssd1306::I2CDisplayInterface::new(i2c);
    let rotation = if OLED_ROTATE_180 {
        DisplayRotation::Rotate180
    } else {
        DisplayRotation::Rotate0
    };
    let mut display =
        ssd1306::Ssd1306::new(interface, DisplaySize128x64, rotation).into_buffered_graphics_mode();

    if display.init().is_err() {
        warn!("display init failed (check wiring, power, and I2C address)");
    }
    let _ = display.set_invert(OLED_INVERT);
    let _ = display.flush();

    info!("rendering test pattern (invert={}, rotate_180={})", OLED_INVERT, OLED_ROTATE_180);

    let style_small = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let style_big = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    let mut frame: u32 = 0;
    loop {
        frame = frame.wrapping_add(1);

        display.clear_buffer();

        // Border.
        let _ = Rectangle::new(Point::new(0, 0), Size::new(128, 64))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut display);

        // Crosshair to help validate rotation/origin.
        let _ = Line::new(Point::new(64, 0), Point::new(64, 63))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut display);
        let _ = Line::new(Point::new(0, 32), Point::new(127, 32))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut display);

        // Title + pin hint.
        let _ = Text::with_baseline(
            "I2C OLED TEST",
            Point::new(6, 2),
            style_small,
            Baseline::Top,
        )
        .draw(&mut display);
        let _ = Text::with_baseline(
            "SDA21 SCL22",
            Point::new(6, 12),
            style_small,
            Baseline::Top,
        )
        .draw(&mut display);

        // Frame counter (big text).
        let mut buf = heapless::String::<16>::new();
        {
            use core::fmt::Write as _;
            let _ = core::write!(&mut buf, "#{:05}", frame);
        }
        let _ = Text::with_baseline(&buf, Point::new(6, 18), style_big, Baseline::Top)
            .draw(&mut display);

        // Moving filled square along the bottom (checks continuous refresh).
        let x = (frame % 120) as i32;
        let _ = Rectangle::new(Point::new(x, 52), Size::new(8, 8))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut display);

        if display.flush().is_err() {
            warn!("display flush failed (I2C error)");
        }

        Timer::after(Duration::from_millis(FRAME_MS)).await;
    }
}
