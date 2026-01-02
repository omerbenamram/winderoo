//! ESP32 runtime integration using ESP-IDF services.
//!
//! This is the Rust port of the Arduino sketch-style `main.cpp`:
//! `src/platformio/osww-server/src/main.cpp`.
//!
//! High-level idea: the pure "business logic" lives in `Controller` and emits events.
//! This module wires those events to ESP32 side-effects (motor/LED/OLED, LittleFS, Wi‑Fi, MQTT).

mod events;
#[cfg(feature = "home-assistant")]
mod ha_mqtt;
mod hardware;
mod http;
#[cfg(feature = "oled")]
mod oled;
mod storage;
mod wifi;

use crate::controller::Controller;
use crate::hardware::{LedPattern, XorShift32};
use crate::settings::SettingsError;
use crate::time::time_of_day_from_epoch;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::fs::littlefs::Littlefs;
use esp_idf_svc::io::vfs::MountedLittlefs;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::mdns::EspMdns;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sntp::{EspSntp, SyncStatus};
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use log::warn;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

#[cfg(feature = "home-assistant")]
use ha_mqtt::HomeAssistant;

use hardware::Hardware;
use storage::Storage;

use wifi::{connect_wifi, start_config_portal, WifiCredentials};

const API_VERSION: &str = "4.0.1";
const HOSTNAME: &str = "winderoo";
const AP_SSID: &str = "Winderoo Setup";
const FS_ROOT: &str = "/littlefs";
const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Error)]
pub enum Esp32Error {
    #[error("esp-idf error: {0}")]
    Esp(#[from] esp_idf_sys::EspError),
    #[error("esp-idf io error: {0}")]
    SvcIo(#[from] esp_idf_svc::io::EspIOError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("settings error: {0}")]
    Settings(#[from] SettingsError),
    #[error("api error: {0}")]
    Api(#[from] crate::api::ApiError),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("lock poisoned")]
    Lock,
    #[cfg(feature = "oled")]
    #[error("display error: {0:?}")]
    Display(display_interface::DisplayError),
}

#[cfg(feature = "oled")]
impl From<display_interface::DisplayError> for Esp32Error {
    fn from(value: display_interface::DisplayError) -> Self {
        Self::Display(value)
    }
}

pub fn run() -> Result<(), Esp32Error> {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let Peripherals {
        pins,
        ledc,
        i2c0,
        modem,
        ..
    } = peripherals;

    let hardware = Hardware::new(pins, ledc, i2c0)?;

    let _mounted_fs = mount_littlefs()?;
    let storage = Storage::new(FS_ROOT, SETTINGS_FILE);
    let stored = storage.load_or_init()?;

    let screen_equipped = cfg!(feature = "oled");
    let runtime = stored.to_runtime(screen_equipped)?;
    let rng_seed = unsafe { esp_idf_sys::esp_random() };

    // Rust replacement for the big set of Arduino globals in `main.cpp`:
    // - controller state (what was `userDefinedSettings`, `routineRunning`, timestamps, etc)
    // - a couple of cross-cutting runtime signals (RSSI + reset request)
    //
    // We put it behind `Arc<Mutex<...>>` because it is touched from:
    // - the main control loop (tick/button)
    // - HTTP handlers (API updates)
    // - MQTT command ingestion (Home Assistant)
    //
    // Keep lock scopes *small* and never call blocking IO (FS/network/delays) while holding it.
    let shared = Arc::new(Mutex::new(SharedState {
        controller: Controller::new(runtime, XorShift32::new(rng_seed)),
        rssi: -100,
        reset_requested: false,
    }));

    // Hardware drivers are not thread-safe and share peripherals (GPIO/I2C/LEDC).
    // The mutex is our "single-threaded peripheral access" gate.
    let hardware = Arc::new(Mutex::new(hardware));
    let storage = Arc::new(storage);

    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(modem, sysloop.clone(), Some(nvs.clone()))?,
        sysloop.clone(),
    )?;

    // Mirrors the C++ WiFiManager flow:
    // try saved credentials; if missing/failing, boot into a setup AP + captive portal.
    let creds = WifiCredentials::load(&nvs)?;
    if let Some(creds) = creds {
        if let Err(err) = connect_wifi(&mut wifi, &creds) {
            warn!("failed to connect to saved wifi: {err:?}");
            start_config_portal(&mut wifi, &storage, &hardware, &shared, &nvs)?;
        }
    } else {
        start_config_portal(&mut wifi, &storage, &hardware, &shared, &nvs)?;
    }

    let mut mdns = EspMdns::take()?;
    mdns.set_hostname(HOSTNAME)?;
    mdns.add_service(Some(HOSTNAME), "_winderoo", "_tcp", 80, &[])?;

    sync_time()?;

    // Start the HTTP server early so the UI can drive configuration/state.
    // Handlers take `Arc` clones and lock `SharedState` only long enough to compute controller events.
    let _server = http::start_http_server(shared.clone(), hardware.clone(), storage.clone())?;

    #[cfg(feature = "home-assistant")]
    let ha = HomeAssistant::try_new(shared.clone(), API_VERSION)?;

    // C++: if `savedStatus == "Winding"` we resume the routine on boot.
    // Rust: let the controller decide what needs to happen based on persisted runtime + current time.
    resume_if_needed(&shared, &hardware, &storage)?;

    run_loop(
        shared,
        hardware,
        storage,
        wifi,
        nvs.clone(),
        #[cfg(feature = "home-assistant")]
        ha,
    )?;

    Ok(())
}

fn mount_littlefs() -> Result<MountedLittlefs<Littlefs<std::ffi::CString>>, Esp32Error> {
    let littlefs = unsafe { Littlefs::new_partition("littlefs")? };
    let mounted = MountedLittlefs::mount(littlefs, FS_ROOT)?;
    Ok(mounted)
}

fn resume_if_needed(
    shared: &Arc<Mutex<SharedState>>,
    hardware: &Arc<Mutex<Hardware>>,
    storage: &Arc<Storage>,
) -> Result<(), Esp32Error> {
    let now = current_epoch();
    let events = {
        // Lock only while mutating controller state.
        let mut guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
        guard.controller.resume_if_needed(now)
    };
    // Apply side-effects after we drop the controller lock (prevents long hardware/FS work from
    // blocking HTTP/MQTT threads).
    events::apply_events(shared, hardware, storage, events)
}

fn run_loop(
    shared: Arc<Mutex<SharedState>>,
    hardware: Arc<Mutex<Hardware>>,
    storage: Arc<Storage>,
    mut wifi: BlockingWifi<EspWifi>,
    nvs: EspDefaultNvsPartition,
    #[cfg(feature = "home-assistant")] mut ha: Option<HomeAssistant>,
) -> Result<(), Esp32Error> {
    let mut last_tick = Instant::now();
    #[cfg(feature = "home-assistant")]
    let mut last_ha_publish = Instant::now();

    loop {
        if last_tick.elapsed() >= Duration::from_secs(1) {
            let epoch = current_epoch();
            let events = {
                let mut guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
                // RSSI is exposed to UI/HA (and used for the OLED "bars").
                // Store it in shared state so *all* publishers see a consistent value.
                guard.rssi = wifi
                    .wifi_mut()
                    .driver_mut()
                    .get_ap_info()
                    .map(|info| info.signal_strength as i32)
                    .unwrap_or(-100);
                let time = time_of_day_from_epoch(
                    epoch,
                    guard.controller.state.rtc.gmt_offset,
                    guard.controller.state.rtc.dst,
                );
                // This is the ported equivalent of `loop()` in C++: the controller advances time
                // and returns a list of side-effects to execute.
                let events = guard.controller.tick(epoch, time);
                events
            };
            events::apply_events(&shared, &hardware, &storage, events)?;
            last_tick = Instant::now();
        }

        if check_reset_requested(&shared)? {
            // C++ kept a `reset` global flag and performed the actual reset from the main loop.
            // We do the same: HTTP/MQTT can *request* a reset, but only the main loop performs it
            // so we can show notifications, flush storage, and restart from a safe context.
            notify_and_restart(&shared, &hardware, &storage, &nvs)?;
        }

        if let Ok(button_pressed) = read_button(&hardware) {
            if button_pressed {
                let events = {
                    // Physical button is treated like a "hard stop" (power off).
                    let mut guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
                    guard.controller.apply_power(false)
                };
                events::apply_events(&shared, &hardware, &storage, events)?;
            }
        }

        #[cfg(feature = "home-assistant")]
        {
            if let Some(ha) = ha.as_mut() {
                if last_ha_publish.elapsed() >= Duration::from_secs(5) {
                    ha.publish_state(&shared)?;
                    last_ha_publish = Instant::now();
                }
                // Commands are collected by the MQTT callback thread and applied here, in-band with
                // the main loop, to keep lock contention predictable.
                ha.drain_commands(&shared, &hardware, &storage)?;
            }
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn read_button(hardware: &Arc<Mutex<Hardware>>) -> Result<bool, Esp32Error> {
    let guard = hardware.lock().map_err(|_| Esp32Error::Lock)?;
    Ok(guard.button_is_high())
}

fn check_reset_requested(shared: &Arc<Mutex<SharedState>>) -> Result<bool, Esp32Error> {
    let guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
    Ok(guard.reset_requested)
}

fn notify_and_restart(
    _shared: &Arc<Mutex<SharedState>>,
    hardware: &Arc<Mutex<Hardware>>,
    storage: &Arc<Storage>,
    nvs: &EspDefaultNvsPartition,
) -> Result<(), Esp32Error> {
    {
        let mut guard = hardware.lock().map_err(|_| Esp32Error::Lock)?;
        guard.display_notification("Resetting")?;
        guard.led_trigger(LedPattern::FastBlink)?;
    }

    // Clear Wi‑Fi creds to force the setup portal next boot (same user-facing behavior as
    // `wm.resetSettings()` in the Arduino firmware).
    WifiCredentials::clear(nvs)?;
    storage.flush()?;

    unsafe { esp_idf_sys::esp_restart() };
    #[allow(unreachable_code)]
    Ok(())
}

fn sync_time() -> Result<(), Esp32Error> {
    let sntp = EspSntp::new_default()?;
    let start = Instant::now();
    while sntp.get_sync_status() != SyncStatus::Completed {
        if start.elapsed() > Duration::from_secs(15) {
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

fn current_epoch() -> u64 {
    unsafe {
        let mut now: esp_idf_sys::time_t = 0;
        esp_idf_sys::time(&mut now as *mut _);
        now as u64
    }
}

struct SharedState {
    // This is intentionally small: most runtime state is owned by `Controller`.
    // Any field added here becomes "global shared mutable state" (like C++ globals), so keep it
    // to cross-cutting telemetry/signals only.
    controller: Controller<XorShift32>,
    rssi: i32,
    reset_requested: bool,
}

// Home Assistant support lives in `esp32/ha_mqtt.rs` and `home_assistant.rs`.
