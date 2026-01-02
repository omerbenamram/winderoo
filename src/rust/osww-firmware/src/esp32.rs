//! ESP32 runtime integration using ESP-IDF services.

#[cfg(feature = "home-assistant")]
mod ha_mqtt;
#[cfg(feature = "oled")]
mod oled;

use crate::api::{PowerPayload, ResetResponse, StatusResponse, UpdatePayload, UpdateRequest};
use crate::controller::{Controller, ControllerEvent};
use crate::hardware::{LedPattern, XorShift32};
use crate::model::{Direction, MotorDirection, RuntimeState, WinderStatus};
use crate::settings::{SettingsError, StoredSettings};
use crate::time::{time_of_day_from_epoch, TimeOfDay};
use embedded_svc::http::headers::content_type;
use embedded_svc::http::Method;
use embedded_svc::io::{Read as SvcRead, Write as SvcWrite};
use esp_idf_hal::gpio::{Input, Output, PinDriver, Pull};
use esp_idf_hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_hal::ledc::{config::TimerConfig, LedcDriver, LedcTimerDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::prelude::*;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::fs::littlefs::Littlefs;
use esp_idf_svc::http::server::{Configuration as HttpConfig, EspHttpServer};
use esp_idf_svc::io::vfs::MountedLittlefs;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::mdns::EspMdns;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs};
use esp_idf_svc::sntp::{EspSntp, SyncStatus};
use esp_idf_svc::wifi::{
    AccessPointConfiguration, AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi,
};
use heapless::String as HeaplessString;
use log::{info, warn};
use std::fs;
use std::io::{Read as StdRead, Write as StdWrite};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

#[cfg(feature = "home-assistant")]
use ha_mqtt::HomeAssistant;

#[cfg(feature = "oled")]
use oled::OledDisplay;

const API_VERSION: &str = "4.0.1";
const HOSTNAME: &str = "winderoo";
const AP_SSID: &str = "Winderoo Setup";
const FS_ROOT: &str = "/littlefs";
const SETTINGS_FILE: &str = "settings.json";

type MotorPinA = esp_idf_hal::gpio::Gpio25;
type MotorPinB = esp_idf_hal::gpio::Gpio26;
type ButtonPin = esp_idf_hal::gpio::Gpio13;

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

    let mut hardware = Hardware::new(pins, ledc, i2c0)?;

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

    let _server = start_http_server(shared.clone(), hardware.clone(), storage.clone())?;

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

fn connect_wifi(
    wifi: &mut BlockingWifi<EspWifi>,
    creds: &WifiCredentials,
) -> Result<(), Esp32Error> {
    let mut client_cfg = ClientConfiguration::default();
    client_cfg.ssid = to_heapless(&creds.ssid)?;
    client_cfg.password = to_heapless(&creds.password)?;
    client_cfg.auth_method = if creds.password.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    };

    wifi.set_configuration(&Configuration::Client(client_cfg))?;
    wifi.start()?;
    wifi.connect()?;
    wifi.wait_netif_up()?;

    info!("connected to wifi");
    Ok(())
}

fn start_config_portal(
    wifi: &mut BlockingWifi<EspWifi>,
    storage: &Arc<Storage>,
    hardware: &Arc<Mutex<Hardware>>,
    shared: &Arc<Mutex<SharedState>>,
    nvs: &EspDefaultNvsPartition,
) -> Result<(), Esp32Error> {
    info!("starting wifi config portal");

    let mut ap_cfg = AccessPointConfiguration::default();
    ap_cfg.ssid = to_heapless(AP_SSID)?;
    ap_cfg.auth_method = AuthMethod::None;
    ap_cfg.password = HeaplessString::new();

    wifi.set_configuration(&Configuration::AccessPoint(ap_cfg))?;
    wifi.start()?;

    let portal_state = Arc::new(Mutex::new(false));
    let portal_state_handler = portal_state.clone();
    let nvs_handler = nvs.clone();

    let mut server = EspHttpServer::new(&HttpConfig {
        uri_match_wildcard: true,
        ..Default::default()
    })?;

    server.fn_handler("/", Method::Get, move |req| -> Result<(), Esp32Error> {
        let page = config_portal_page();
        let headers = [content_type("text/html"), cors_allow_origin()];
        let mut response = req.into_response(200, Some("OK"), &headers)?;
        response.write_all(page.as_bytes())?;
        Ok(())
    })?;

    server.fn_handler("/*", Method::Get, move |req| -> Result<(), Esp32Error> {
        let page = config_portal_page();
        let headers = [content_type("text/html"), cors_allow_origin()];
        let mut response = req.into_response(200, Some("OK"), &headers)?;
        response.write_all(page.as_bytes())?;
        Ok(())
    })?;

    server.fn_handler(
        "/wifi",
        Method::Post,
        move |mut req| -> Result<(), Esp32Error> {
            let body = read_request_body(&mut req)?;

            if let Some((ssid, password)) = parse_wifi_payload(&body) {
                let creds = WifiCredentials { ssid, password };
                WifiCredentials::save(&nvs_handler, &creds).map_err(Esp32Error::from)?;
                let headers = [content_type("text/plain"), cors_allow_origin()];
                let mut response = req.into_response(200, Some("OK"), &headers)?;
                response.write_all(b"Saved. Restarting...")?;

                let mut flag = portal_state_handler.lock().map_err(|_| Esp32Error::Lock)?;
                *flag = true;
                return Ok(());
            }

            let headers = [content_type("text/plain"), cors_allow_origin()];
            let mut response = req.into_response(400, Some("Bad Request"), &headers)?;
            response.write_all(b"Invalid payload")?;
            Ok(())
        },
    )?;

    loop {
        if *portal_state.lock().map_err(|_| Esp32Error::Lock)? {
            notify_and_restart(shared, hardware, storage, nvs)?;
        }
        thread::sleep(Duration::from_millis(200));
    }
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
    let mut guard = hardware.lock().map_err(|_| Esp32Error::Lock)?;
    Ok(guard.button.is_high())
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
            ControllerEvent::MotorStart(dir) => hw.motor.start(dir)?,
            ControllerEvent::MotorStop => hw.motor.stop()?,
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
            ControllerEvent::Led(pattern) => hw.led.trigger(pattern)?,
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
    shared: &Arc<Mutex<SharedState>>,
    hardware: &Arc<Mutex<Hardware>>,
    storage: &Arc<Storage>,
    nvs: &EspDefaultNvsPartition,
) -> Result<(), Esp32Error> {
    {
        let mut guard = hardware.lock().map_err(|_| Esp32Error::Lock)?;
        guard.display_notification("Resetting")?;
        guard.led.trigger(LedPattern::FastBlink)?;
    }

    WifiCredentials::clear(nvs)?;
    storage.flush()?;

    unsafe { esp_idf_sys::esp_restart() };
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

fn start_http_server(
    shared: Arc<Mutex<SharedState>>,
    hardware: Arc<Mutex<Hardware>>,
    storage: Arc<Storage>,
) -> Result<EspHttpServer<'static>, Esp32Error> {
    let mut server = EspHttpServer::new(&HttpConfig {
        uri_match_wildcard: true,
        max_resp_headers: 16,
        ..Default::default()
    })?;

    let shared_status = shared.clone();
    server.fn_handler(
        "/api/status",
        Method::Get,
        move |req| -> Result<(), Esp32Error> {
            let (status, rssi) = {
                let guard = shared_status.lock().map_err(|_| Esp32Error::Lock)?;
                let response =
                    guard
                        .controller
                        .status_response(current_epoch(), guard.rssi, API_VERSION);
                (response, guard.rssi)
            };
            let body = serde_json::to_string(&status)?;
            respond_json(req, 200, &body)?;
            let _ = rssi;
            Ok(())
        },
    )?;

    let shared_timer = shared.clone();
    let hardware_timer = hardware.clone();
    let storage_timer = storage.clone();
    server.fn_handler(
        "/api/timer",
        Method::Post,
        move |req| -> Result<(), Esp32Error> {
            let timer_enabled = parse_query_bool(req.uri(), "timerEnabled").unwrap_or(false);
            let events = {
                let mut guard = shared_timer.lock().map_err(|_| Esp32Error::Lock)?;
                guard.controller.apply_timer_enabled(timer_enabled)
            };
            apply_events(&shared_timer, &hardware_timer, &storage_timer, events)?;
            respond_empty(req, 204)?;
            Ok(())
        },
    )?;

    let shared_power = shared.clone();
    let hardware_power = hardware.clone();
    let storage_power = storage.clone();
    server.fn_handler(
        "/api/power",
        Method::Post,
        move |mut req| -> Result<(), Esp32Error> {
            let body = read_request_body(&mut req)?;
            let payload: PowerPayload = serde_json::from_str(&body)?;
            let events = {
                let mut guard = shared_power.lock().map_err(|_| Esp32Error::Lock)?;
                guard.controller.apply_power(payload.winder_enabled)
            };
            apply_events(&shared_power, &hardware_power, &storage_power, events)?;
            respond_empty(req, 204)?;
            Ok(())
        },
    )?;

    let shared_update = shared.clone();
    let hardware_update = hardware.clone();
    let storage_update = storage.clone();
    server.fn_handler(
        "/api/update",
        Method::Post,
        move |mut req| -> Result<(), Esp32Error> {
            let body = read_request_body(&mut req)?;
            let payload: UpdatePayload = serde_json::from_str(&body)?;
            let update: UpdateRequest = payload.try_into()?;
            let events = {
                let mut guard = shared_update.lock().map_err(|_| Esp32Error::Lock)?;
                guard.controller.apply_update(update, current_epoch())
            };
            apply_events(&shared_update, &hardware_update, &storage_update, events)?;
            respond_empty(req, 204)?;
            Ok(())
        },
    )?;

    let shared_reset = shared.clone();
    let hardware_reset = hardware.clone();
    let storage_reset = storage.clone();
    server.fn_handler(
        "/api/reset",
        Method::Get,
        move |req| -> Result<(), Esp32Error> {
            let response = ResetResponse::new();
            let body = serde_json::to_string(&response)?;
            respond_json(req, 200, &body)?;
            let events = {
                let mut guard = shared_reset.lock().map_err(|_| Esp32Error::Lock)?;
                guard.controller.request_reset()
            };
            apply_events(&shared_reset, &hardware_reset, &storage_reset, events)?;
            Ok(())
        },
    )?;

    let static_storage = storage.clone();
    server.fn_handler("/*", Method::Get, move |req| -> Result<(), Esp32Error> {
        serve_static(req, &static_storage)
    })?;

    let static_storage_options = storage.clone();
    server.fn_handler(
        "/*",
        Method::Options,
        move |req| -> Result<(), Esp32Error> {
            let _ = static_storage_options;
            respond_empty(req, 200)?;
            Ok(())
        },
    )?;

    Ok(server)
}

fn respond_json<C>(
    req: embedded_svc::http::server::Request<C>,
    status: u16,
    body: &str,
) -> Result<(), Esp32Error>
where
    C: embedded_svc::http::server::Connection,
    Esp32Error: From<<C as embedded_svc::io::ErrorType>::Error>,
{
    let headers = [
        content_type("application/json"),
        cors_allow_origin(),
        cors_allow_methods(),
        cors_allow_headers(),
    ];
    let mut response = req.into_response(status, Some("OK"), &headers)?;
    response.write_all(body.as_bytes())?;
    Ok(())
}

fn respond_empty<C>(
    req: embedded_svc::http::server::Request<C>,
    status: u16,
) -> Result<(), Esp32Error>
where
    C: embedded_svc::http::server::Connection,
    Esp32Error: From<<C as embedded_svc::io::ErrorType>::Error>,
{
    let headers = [
        cors_allow_origin(),
        cors_allow_methods(),
        cors_allow_headers(),
    ];
    let mut response = req.into_response(status, Some("OK"), &headers)?;
    response.write_all(&[])?;
    Ok(())
}

fn serve_static<C>(
    req: embedded_svc::http::server::Request<C>,
    storage: &Storage,
) -> Result<(), Esp32Error>
where
    C: embedded_svc::http::server::Connection,
    Esp32Error: From<<C as embedded_svc::io::ErrorType>::Error>,
{
    let path = storage.resolve_asset(req.uri());
    match path {
        Some(asset) => {
            let mut headers = vec![
                content_type(asset.content_type),
                cors_allow_origin(),
                cors_allow_methods(),
                cors_allow_headers(),
                cache_control(asset.cache_control),
            ];
            if !asset.content_encoding.is_empty() {
                headers.push(("Content-Encoding", asset.content_encoding));
            }
            let mut response = req.into_response(200, Some("OK"), &headers)?;
            response.write_all(&asset.bytes)?;
            Ok(())
        }
        None => {
            let headers = [content_type("text/plain"), cors_allow_origin()];
            let mut response = req.into_response(404, Some("Not Found"), &headers)?;
            response.write_all(b"Winderoo\n\n404 - Resource Not found")?;
            Ok(())
        }
    }
}

fn cors_allow_origin() -> (&'static str, &'static str) {
    ("Access-Control-Allow-Origin", "*")
}

fn cors_allow_methods() -> (&'static str, &'static str) {
    ("Access-Control-Allow-Methods", "GET,POST,OPTIONS")
}

fn cors_allow_headers() -> (&'static str, &'static str) {
    (
        "Access-Control-Allow-Headers",
        "Content-Type, Access-Control-Allow-Headers, Authorization, X-Requested-With",
    )
}

fn cache_control(value: &'static str) -> (&'static str, &'static str) {
    ("Cache-Control", value)
}

fn read_request_body<C>(
    req: &mut embedded_svc::http::server::Request<C>,
) -> Result<String, Esp32Error>
where
    C: embedded_svc::http::server::Connection,
    Esp32Error: From<<C as embedded_svc::io::ErrorType>::Error>,
{
    let mut body = String::new();
    let mut buf = [0u8; 1024];

    loop {
        let read = req.read(&mut buf)?;
        if read == 0 {
            break;
        }

        let chunk = std::str::from_utf8(&buf[..read]).map_err(|err| {
            Esp32Error::InvalidConfig(format!("invalid request body utf-8: {err}"))
        })?;
        body.push_str(chunk);
    }

    Ok(body)
}

fn parse_query_bool(uri: &str, key: &str) -> Option<bool> {
    let query = uri.split('?').nth(1)?;
    for part in query.split('&') {
        let mut iter = part.split('=');
        if let (Some(k), Some(v)) = (iter.next(), iter.next()) {
            if k == key {
                return Some(matches!(v, "1" | "true" | "True" | "TRUE"));
            }
        }
    }
    None
}

fn parse_wifi_payload(body: &str) -> Option<(String, String)> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        let ssid = json.get("ssid")?.as_str()?.to_string();
        let password = json
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        return Some((ssid, password));
    }

    let mut ssid = None;
    let mut password = None;
    for part in body.split('&') {
        let mut iter = part.split('=');
        if let (Some(k), Some(v)) = (iter.next(), iter.next()) {
            if k == "ssid" {
                ssid = Some(v.to_string());
            } else if k == "password" {
                password = Some(v.to_string());
            }
        }
    }

    ssid.map(|s| (s, password.unwrap_or_default()))
}

fn config_portal_page() -> &'static str {
    r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Winderoo WiFi Setup</title>
  <style>
    body { font-family: Arial, sans-serif; padding: 24px; }
    label { display: block; margin-top: 12px; }
    input { width: 100%; padding: 8px; font-size: 16px; }
    button { margin-top: 16px; padding: 10px 16px; font-size: 16px; }
  </style>
</head>
<body>
  <h2>Winderoo WiFi Setup</h2>
  <p>Enter your WiFi credentials to connect this device.</p>
  <form method="post" action="/wifi">
    <label>SSID</label>
    <input name="ssid" required />
    <label>Password</label>
    <input name="password" type="password" />
    <button type="submit">Save & Restart</button>
  </form>
</body>
</html>"#
}

fn to_heapless<const N: usize>(value: &str) -> Result<HeaplessString<N>, Esp32Error> {
    HeaplessString::try_from(value).map_err(|_| Esp32Error::InvalidConfig(value.to_string()))
}

#[derive(Debug, Clone)]
struct WifiCredentials {
    ssid: String,
    password: String,
}

impl WifiCredentials {
    fn load(nvs: &EspDefaultNvsPartition) -> Result<Option<Self>, Esp32Error> {
        let nvs = EspNvs::new(nvs.clone(), "wifi", true)?;
        let mut ssid_buf = [0u8; 33];
        let mut pass_buf = [0u8; 65];
        let ssid = nvs.get_str("ssid", &mut ssid_buf)?;
        let password = nvs.get_str("password", &mut pass_buf)?;
        match ssid {
            Some(ssid) => Ok(Some(Self {
                ssid: ssid.to_string(),
                password: password.unwrap_or("").to_string(),
            })),
            None => Ok(None),
        }
    }

    fn save(nvs: &EspDefaultNvsPartition, creds: &WifiCredentials) -> Result<(), Esp32Error> {
        let mut nvs = EspNvs::new(nvs.clone(), "wifi", true)?;
        nvs.set_str("ssid", &creds.ssid)?;
        nvs.set_str("password", &creds.password)?;
        Ok(())
    }

    fn clear(nvs: &EspDefaultNvsPartition) -> Result<(), Esp32Error> {
        let mut nvs = EspNvs::new(nvs.clone(), "wifi", true)?;
        let _ = nvs.remove("ssid");
        let _ = nvs.remove("password");
        Ok(())
    }
}

struct Storage {
    root: PathBuf,
    settings_path: PathBuf,
}

impl Storage {
    fn new(root: &str, settings_file: &str) -> Self {
        let root = PathBuf::from(root);
        let settings_path = root.join(settings_file);
        Self {
            root,
            settings_path,
        }
    }

    fn load_or_init(&self) -> Result<StoredSettings, Esp32Error> {
        if let Ok(contents) = fs::read_to_string(&self.settings_path) {
            if let Ok(settings) = serde_json::from_str::<StoredSettings>(&contents) {
                return Ok(settings);
            }
        }
        let settings = StoredSettings::default();
        self.save(&settings)?;
        Ok(settings)
    }

    fn save(&self, settings: &StoredSettings) -> Result<(), Esp32Error> {
        let json = serde_json::to_string_pretty(settings)?;
        fs::write(&self.settings_path, json)?;
        Ok(())
    }

    fn flush(&self) -> Result<(), Esp32Error> {
        Ok(())
    }

    fn resolve_asset(&self, uri: &str) -> Option<StaticAsset> {
        let mut path = uri.split('?').next().unwrap_or("").trim_start_matches('/');
        if path.is_empty() {
            path = "index.html";
        }
        if path.contains("..") {
            return None;
        }

        let candidate = self.root.join(path);
        if let Some(asset) = StaticAsset::from_path(&candidate, false) {
            return Some(asset);
        }

        let gz_candidate = PathBuf::from(format!("{}.gz", candidate.display()));
        if let Some(asset) = StaticAsset::from_path(&gz_candidate, true) {
            return Some(asset);
        }

        None
    }
}

struct StaticAsset {
    bytes: Vec<u8>,
    content_type: &'static str,
    cache_control: &'static str,
    content_encoding: &'static str,
}

impl StaticAsset {
    fn from_path(path: &Path, gzipped: bool) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        let ext = if gzipped {
            path.file_stem()
                .and_then(|s| Path::new(s).extension())
                .and_then(|s| s.to_str())
                .unwrap_or("")
        } else {
            path.extension().and_then(|s| s.to_str()).unwrap_or("")
        };
        let content_type = match ext {
            "html" | "htm" => "text/html",
            "css" => "text/css",
            "js" => "application/javascript",
            "json" => "application/json",
            "ico" => "image/x-icon",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gz" => "application/octet-stream",
            _ => "application/octet-stream",
        };
        let cache_control = if ext == "js" || ext == "css" {
            "max-age=31536000"
        } else {
            "no-cache"
        };
        let content_encoding = if gzipped { "gzip" } else { "" };
        Some(Self {
            bytes,
            content_type,
            cache_control,
            content_encoding,
        })
    }
}

struct SharedState {
    controller: Controller<XorShift32>,
    rssi: i32,
    reset_requested: bool,
}

struct Hardware {
    motor: MotorControl,
    led: LedControl,
    button: PinDriver<'static, ButtonPin, Input>,
    #[cfg(feature = "oled")]
    display: Option<OledDisplay>,
}

impl Hardware {
    fn new(
        pins: esp_idf_hal::gpio::Pins,
        ledc: esp_idf_hal::ledc::LEDC,
        i2c0: esp_idf_hal::i2c::I2C0,
    ) -> Result<Self, Esp32Error> {
        let mut pins = pins;
        let ledc = ledc;
        let _i2c0 = i2c0;

        let esp_idf_hal::ledc::LEDC {
            timer0,
            timer1,
            channel0,
            channel1,
            channel2,
            ..
        } = ledc;

        let led_timer =
            LedcTimerDriver::new(timer0, &TimerConfig::default().frequency(5.kHz().into()))?;
        let led_driver = LedcDriver::new(channel0, &led_timer, pins.gpio2)?;
        let led = LedControl::new(led_timer, led_driver);

        let motor = MotorControl::new(
            pins.gpio25,
            pins.gpio26,
            #[cfg(feature = "pwm-motor")]
            timer1,
            #[cfg(feature = "pwm-motor")]
            channel1,
            #[cfg(feature = "pwm-motor")]
            channel2,
        )?;

        let mut button = PinDriver::input(pins.gpio13)?;
        button.set_pull(Pull::Down)?;

        #[cfg(feature = "oled")]
        let display = {
            let config = I2cConfig::new().baudrate(400.kHz().into());
            let i2c = I2cDriver::new(_i2c0, pins.gpio21, pins.gpio22, &config)?;
            Some(OledDisplay::new(i2c)?)
        };

        Ok(Self {
            motor,
            led,
            button,
            #[cfg(feature = "oled")]
            display,
        })
    }

    fn display_clear(&mut self) -> Result<(), Esp32Error> {
        #[cfg(feature = "oled")]
        if let Some(display) = &mut self.display {
            display.clear()?;
        }
        Ok(())
    }

    fn display_static(&mut self, title: &str) -> Result<(), Esp32Error> {
        #[cfg(feature = "oled")]
        if let Some(display) = &mut self.display {
            display.draw_static(title)?;
        }
        Ok(())
    }

    fn display_dynamic(&mut self, state: &RuntimeState, rssi: i32) -> Result<(), Esp32Error> {
        #[cfg(feature = "oled")]
        if let Some(display) = &mut self.display {
            if !state.screen.sleep {
                display.draw_dynamic(state, rssi)?;
            }
        }
        Ok(())
    }

    fn display_notification(&mut self, message: &str) -> Result<(), Esp32Error> {
        #[cfg(feature = "oled")]
        if let Some(display) = &mut self.display {
            display.draw_notification(message)?;
        }
        Ok(())
    }
}

struct MotorControl {
    #[cfg(not(feature = "pwm-motor"))]
    pin_a: PinDriver<'static, MotorPinA, Output>,
    #[cfg(not(feature = "pwm-motor"))]
    pin_b: PinDriver<'static, MotorPinB, Output>,
    #[cfg(feature = "pwm-motor")]
    pwm: MotorPwm,
}

impl MotorControl {
    fn new(
        pin_a: MotorPinA,
        pin_b: MotorPinB,
        #[cfg(feature = "pwm-motor")] timer1: esp_idf_hal::ledc::TIMER1,
        #[cfg(feature = "pwm-motor")] channel1: esp_idf_hal::ledc::CHANNEL1,
        #[cfg(feature = "pwm-motor")] channel2: esp_idf_hal::ledc::CHANNEL2,
    ) -> Result<Self, Esp32Error> {
        #[cfg(not(feature = "pwm-motor"))]
        let pin_a = PinDriver::output(pin_a)?;
        #[cfg(not(feature = "pwm-motor"))]
        let pin_b = PinDriver::output(pin_b)?;

        #[cfg(feature = "pwm-motor")]
        let pwm = MotorPwm::new(timer1, channel1, channel2, pin_a, pin_b)?;

        #[cfg(feature = "pwm-motor")]
        {
            Ok(Self { pwm })
        }

        #[cfg(not(feature = "pwm-motor"))]
        {
            Ok(Self { pin_a, pin_b })
        }
    }

    fn start(&mut self, direction: MotorDirection) -> Result<(), Esp32Error> {
        #[cfg(feature = "pwm-motor")]
        {
            return self.pwm.start(direction);
        }

        #[cfg(not(feature = "pwm-motor"))]
        {
            match direction {
                MotorDirection::Clockwise => {
                    self.pin_a.set_high()?;
                    self.pin_b.set_low()?;
                }
                MotorDirection::CounterClockwise => {
                    self.pin_a.set_low()?;
                    self.pin_b.set_high()?;
                }
            }
            Ok(())
        }
    }

    fn stop(&mut self) -> Result<(), Esp32Error> {
        #[cfg(feature = "pwm-motor")]
        {
            return self.pwm.stop();
        }

        #[cfg(not(feature = "pwm-motor"))]
        {
            self.pin_a.set_low()?;
            self.pin_b.set_low()?;
            Ok(())
        }
    }
}

#[cfg(feature = "pwm-motor")]
struct MotorPwm {
    _timer: LedcTimerDriver<'static, esp_idf_hal::ledc::TIMER1>,
    driver_a: LedcDriver<'static>,
    driver_b: LedcDriver<'static>,
    speed: u32,
}

#[cfg(feature = "pwm-motor")]
impl MotorPwm {
    fn new(
        timer1: esp_idf_hal::ledc::TIMER1,
        channel1: esp_idf_hal::ledc::CHANNEL1,
        channel2: esp_idf_hal::ledc::CHANNEL2,
        pin_a: MotorPinA,
        pin_b: MotorPinB,
    ) -> Result<Self, Esp32Error> {
        let timer =
            LedcTimerDriver::new(timer1, &TimerConfig::default().frequency(1.kHz().into()))?;
        let driver_a = LedcDriver::new(channel1, &timer, pin_a)?;
        let driver_b = LedcDriver::new(channel2, &timer, pin_b)?;
        Ok(Self {
            _timer: timer,
            driver_a,
            driver_b,
            speed: 145,
        })
    }

    fn start(&mut self, direction: MotorDirection) -> Result<(), Esp32Error> {
        let max = self.driver_a.get_max_duty();
        let duty = (self.speed.min(255) as u32 * max) / 255;
        match direction {
            MotorDirection::Clockwise => {
                self.driver_a.set_duty(duty)?;
                self.driver_b.set_duty(0)?;
            }
            MotorDirection::CounterClockwise => {
                self.driver_a.set_duty(0)?;
                self.driver_b.set_duty(duty)?;
            }
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), Esp32Error> {
        self.driver_a.set_duty(0)?;
        self.driver_b.set_duty(0)?;
        Ok(())
    }
}

struct LedControl {
    _timer: LedcTimerDriver<'static, esp_idf_hal::ledc::TIMER0>,
    driver: LedcDriver<'static>,
    max_duty: u32,
}

impl LedControl {
    fn new(
        timer: LedcTimerDriver<'static, esp_idf_hal::ledc::TIMER0>,
        driver: LedcDriver<'static>,
    ) -> Self {
        let max_duty = driver.get_max_duty();
        Self {
            _timer: timer,
            driver,
            max_duty,
        }
    }

    fn trigger(&mut self, pattern: LedPattern) -> Result<(), Esp32Error> {
        self.off()?;
        thread::sleep(Duration::from_millis(50));
        match pattern {
            LedPattern::Off => self.off(),
            LedPattern::SlowBlink => self.slow_blink(),
            LedPattern::FastBlink => self.fast_blink(),
            LedPattern::Pulse => self.pulse(),
        }
    }

    fn off(&mut self) -> Result<(), Esp32Error> {
        self.driver.set_duty(0)?;
        Ok(())
    }

    fn pulse(&mut self) -> Result<(), Esp32Error> {
        for duty in 0..=255 {
            let scaled = (duty as u32 * self.max_duty) / 255;
            self.driver.set_duty(scaled)?;
            thread::sleep(Duration::from_millis(7));
        }
        for duty in (0..=255).rev() {
            let scaled = (duty as u32 * self.max_duty) / 255;
            self.driver.set_duty(scaled)?;
            thread::sleep(Duration::from_millis(7));
        }
        Ok(())
    }

    fn slow_blink(&mut self) -> Result<(), Esp32Error> {
        for _ in 0..3 {
            self.pulse()?;
            thread::sleep(Duration::from_millis(150));
        }
        Ok(())
    }

    fn fast_blink(&mut self) -> Result<(), Esp32Error> {
        for _ in 0..12 {
            for duty in 0..=255 {
                let scaled = (duty as u32 * self.max_duty) / 255;
                self.driver.set_duty(scaled)?;
                thread::sleep(Duration::from_millis(2));
            }
            for duty in (0..=255).rev() {
                let scaled = (duty as u32 * self.max_duty) / 255;
                self.driver.set_duty(scaled)?;
                thread::sleep(Duration::from_millis(2));
            }
            thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }
}

// Home Assistant support lives in `esp32/ha_mqtt.rs` and `home_assistant.rs`.
