//! ESP32 runtime integration using ESP-IDF services.

#[cfg(feature = "home-assistant")]
mod ha_mqtt;
mod http;
mod hardware;
#[cfg(feature = "oled")]
mod oled;
mod storage;
mod wifi;

use crate::controller::{Controller, ControllerEvent};
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

    let shared = Arc::new(Mutex::new(SharedState {
        controller: Controller::new(runtime, XorShift32::new(rng_seed)),
        rssi: -100,
        reset_requested: false,
    }));

    let hardware = Arc::new(Mutex::new(hardware));
    let storage = Arc::new(storage);

    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(modem, sysloop.clone(), Some(nvs.clone()))?,
        sysloop.clone(),
    )?;

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

    let _server = http::start_http_server(shared.clone(), hardware.clone(), storage.clone())?;

    #[cfg(feature = "home-assistant")]
    let ha = HomeAssistant::try_new(shared.clone(), API_VERSION)?;

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
        let mut guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
        guard.controller.resume_if_needed(now)
    };
    apply_events(shared, hardware, storage, events)
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
    let mut last_ha_publish = Instant::now();

    loop {
        if last_tick.elapsed() >= Duration::from_secs(1) {
            let epoch = current_epoch();
            let events = {
                let mut guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
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
                let events = guard.controller.tick(epoch, time);
                events
            };
            apply_events(&shared, &hardware, &storage, events)?;
            last_tick = Instant::now();
        }

        if check_reset_requested(&shared)? {
            notify_and_restart(&shared, &hardware, &storage, &nvs)?;
        }

        if let Ok(button_pressed) = read_button(&hardware) {
            if button_pressed {
                let events = {
                    let mut guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
                    guard.controller.apply_power(false)
                };
                apply_events(&shared, &hardware, &storage, events)?;
            }
        }

        #[cfg(feature = "home-assistant")]
        {
            if let Some(ha) = ha.as_mut() {
                if last_ha_publish.elapsed() >= Duration::from_secs(5) {
                    ha.publish_state(&shared)?;
                    last_ha_publish = Instant::now();
                }
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

fn apply_events(
    shared: &Arc<Mutex<SharedState>>,
    hardware: &Arc<Mutex<Hardware>>,
    storage: &Arc<Storage>,
    events: Vec<ControllerEvent>,
) -> Result<(), Esp32Error> {
    if events.is_empty() {
        return Ok(());
    }

    let mut hw = hardware.lock().map_err(|_| Esp32Error::Lock)?;

    for event in events {
        match event {
            ControllerEvent::MotorStart(dir) => hw.motor_start(dir)?,
            ControllerEvent::MotorStop => hw.motor_stop()?,
            ControllerEvent::PauseSeconds(secs) => {
                thread::sleep(Duration::from_secs(secs as u64));
            }
            ControllerEvent::DisplayClear => hw.display_clear()?,
            ControllerEvent::DisplayStatic { title } => {
                hw.display_static(&title)?;
            }
            ControllerEvent::DisplayDynamic => {
                let guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
                hw.display_dynamic(&guard.controller.state, guard.rssi)?;
            }
            ControllerEvent::DisplayNotification(message) => {
                hw.display_notification(&message)?;
            }
            ControllerEvent::Led(pattern) => hw.led_trigger(pattern)?,
            ControllerEvent::PersistSettings(settings) => storage.save(&settings)?,
            ControllerEvent::SyncTime => {
                if let Err(err) = sync_time() {
                    warn!("time sync failed: {err}");
                }
            }
            ControllerEvent::RestartDevice => {
                let mut guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
                guard.reset_requested = true;
            }
        }
    }

    Ok(())
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
    controller: Controller<XorShift32>,
    rssi: i32,
    reset_requested: bool,
}

// Home Assistant support lives in `esp32/ha_mqtt.rs` and `home_assistant.rs`.
