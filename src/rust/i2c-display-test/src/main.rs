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
    ascii::FONT_6X10,
    MonoTextStyle,
};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Baseline, Text};

const I2C_FREQ_KHZ: u32 = 400;
const OLED_INVERT: bool = false;
const OLED_ROTATE_180: bool = false;
const FRAME_MS: u64 = 40;

const DISPLAY_W: i32 = 128;
const DISPLAY_H: i32 = 64;

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

    info!(
        "rendering DVD bounce (invert={}, rotate_180={})",
        OLED_INVERT, OLED_ROTATE_180
    );

    let style_small = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    // "DVD" logo box (monospace font + padding).
    const LOGO_TEXT: &str = "DVD";
    const PAD: i32 = 2;
    let char_w = FONT_6X10.character_size.width as i32;
    let char_h = FONT_6X10.character_size.height as i32;
    let text_w = (LOGO_TEXT.as_bytes().len() as i32) * char_w;
    let text_h = char_h;
    let box_w = text_w + PAD * 2;
    let box_h = text_h + PAD * 2;

    // Keep the logo inside the border.
    let min_x = 1;
    let min_y = 1;
    let max_x = (DISPLAY_W - 1) - box_w; // x + box_w - 1 <= 126
    let max_y = (DISPLAY_H - 1) - box_h; // y + box_h - 1 <= 62

    let mut x: i32 = 10;
    let mut y: i32 = 10;
    let mut vx: i32 = 2;
    let mut vy: i32 = 1;
    let mut invert_logo = false;

    loop {
        // Update position (bounce off edges).
        let mut bounced = false;

        let mut next_x = x + vx;
        if next_x < min_x {
            next_x = min_x;
            vx = -vx;
            bounced = true;
        } else if next_x > max_x {
            next_x = max_x;
            vx = -vx;
            bounced = true;
        }

        let mut next_y = y + vy;
        if next_y < min_y {
            next_y = min_y;
            vy = -vy;
            bounced = true;
        } else if next_y > max_y {
            next_y = max_y;
            vy = -vy;
            bounced = true;
        }

        x = next_x;
        y = next_y;

        if bounced {
            invert_logo = !invert_logo;
        }

        display.clear_buffer();

        // Border (keeps the bounce constrained and makes rotation obvious).
        let _ = Rectangle::new(Point::new(0, 0), Size::new(DISPLAY_W as u32, DISPLAY_H as u32))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut display);

        // "DVD" logo (simple but very readable on monochrome OLEDs).
        let logo_box = Rectangle::new(Point::new(x, y), Size::new(box_w as u32, box_h as u32));
        if invert_logo {
            // Filled box + "cut out" text.
            let _ = logo_box
                .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                .draw(&mut display);

            let style_inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
            let _ = Text::with_baseline(
                LOGO_TEXT,
                Point::new(x + PAD, y + PAD),
                style_inv,
                Baseline::Top,
            )
            .draw(&mut display);
        } else {
            // Outline + normal text.
            let _ = logo_box
                .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                .draw(&mut display);
            let _ = Text::with_baseline(
                LOGO_TEXT,
                Point::new(x + PAD, y + PAD),
                style_small,
                Baseline::Top,
            )
            .draw(&mut display);
        }

        if display.flush().is_err() {
            warn!("display flush failed (I2C error)");
        }

        Timer::after(Duration::from_millis(FRAME_MS)).await;
    }
}
