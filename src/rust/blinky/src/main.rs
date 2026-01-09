//! ESP32 Blinky - Minimal green LED example using esp-hal + embassy.
//!
//! Adjust `LED_PIN` below to match your board. Common options:
//! - GPIO2: Built-in LED on many ESP32 dev boards
//! - GPIO0: Winderoo status LED
//! - GPIO5, GPIO18, GPIO19: Common dev board choices

#![no_std]
#![no_main]

// Embed ESP-IDF app descriptor for bootloader compatibility.
esp_bootloader_esp_idf::esp_app_desc!();

// Keep panic + exception handlers linked in.
use esp_backtrace as _;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::timer::timg::TimerGroup;
use esp_hal::Config as HalConfig;
use esp_println::logger::init_logger;
use log::info;

/// Timestamp provider for esp-println logger (milliseconds since boot).
#[no_mangle]
pub extern "Rust" fn _esp_println_timestamp() -> u64 {
    esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_millis()
}

#[embassy_executor::task]
async fn blink_task(mut led: Output<'static>) -> ! {
    info!("blink_task: starting LED blink loop on GPIO4");

    let mut state = false;
    loop {
        state = !state;
        if state {
            led.set_high();
            info!("LED ON");
        } else {
            led.set_low();
            info!("LED OFF");
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // Initialize serial logger as early as possible.
    init_logger(log::LevelFilter::Info);
    info!("blinky v{} starting...", env!("CARGO_PKG_VERSION"));

    // Initialize HAL with default clock.
    info!("initializing HAL");
    let config = HalConfig::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    info!("HAL initialized (cpu_clock=max)");

    // Start RTOS scheduler + embassy time driver (TIMG0.timer0).
    info!("starting RTOS scheduler + embassy time driver");
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    info!("RTOS started");

    // Configure LED pin as output.
    // Using GPIO4 (D4 pin) for external LED - GPIO2 is the onboard blue LED.
    info!("configuring GPIO4 as LED output");
    let led = Output::new(peripherals.GPIO4, Level::Low, OutputConfig::default());

    // Spawn the blink task.
    info!("spawning blink task");
    spawner
        .spawn(blink_task(led))
        .expect("failed to spawn blink_task");

    info!("main loop idle");
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
