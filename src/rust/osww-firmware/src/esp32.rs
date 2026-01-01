//! ESP32 runtime integration using ESP-IDF services.

use crate::api::{PowerPayload, ResetResponse, StatusResponse, UpdatePayload, UpdateRequest};
use crate::controller::{calculate_winding_duration_secs, Controller, ControllerEvent};
use crate::hardware::{LedPattern, XorShift32};
use crate::model::{Direction, MotorDirection, RuntimeState, WinderStatus};
use crate::settings::{SettingsError, StoredSettings};
use crate::time::{time_of_day_from_epoch, TimeOfDay};
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
use esp_idf_svc::wifi::{AccessPointConfiguration, AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};
use embedded_svc::http::headers::content_type;
use embedded_svc::http::Method;
use heapless::String as HeaplessString;
use log::{info, warn};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

#[cfg(feature = "oled")]
use display_interface_i2c::I2CInterface;
#[cfg(feature = "oled")]
use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_6X10};
#[cfg(feature = "oled")]
use embedded_graphics::mono_font::MonoTextStyleBuilder;
#[cfg(feature = "oled")]
use embedded_graphics::pixelcolor::BinaryColor;
#[cfg(feature = "oled")]
use embedded_graphics::prelude::*;
#[cfg(feature = "oled")]
use embedded_graphics::primitives::{Line, Rectangle, Triangle, PrimitiveStyle};
#[cfg(feature = "oled")]
use embedded_graphics::text::{Alignment, Text};
#[cfg(feature = "oled")]
use ssd1306::prelude::{DisplayRotation, DisplaySize128x64};
#[cfg(feature = "oled")]
use ssd1306::{Ssd1306, I2CDisplayInterface};

#[cfg(feature = "home-assistant")]
use embedded_svc::mqtt::client::QoS;
#[cfg(feature = "home-assistant")]
use esp_idf_svc::mqtt::client::{EspMqttClient, EventPayload, MqttClientConfiguration};

#[cfg(feature = "home-assistant")]
const HA_BROKER_ENV: Option<&str> = option_env!("HOME_ASSISTANT_BROKER");
#[cfg(feature = "home-assistant")]
const HA_USERNAME_ENV: Option<&str> = option_env!("HOME_ASSISTANT_USERNAME");
#[cfg(feature = "home-assistant")]
const HA_PASSWORD_ENV: Option<&str> = option_env!("HOME_ASSISTANT_PASSWORD");

const API_VERSION: &str = "4.0.1";
const HOSTNAME: &str = "winderoo";
const AP_SSID: &str = "Winderoo Setup";
const FS_ROOT: &str = "/littlefs";
const SETTINGS_FILE: &str = "settings.json";
const OLED_ADDR: u8 = 0x3C;

type MotorPinA = esp_idf_hal::gpio::Gpio25;
type MotorPinB = esp_idf_hal::gpio::Gpio26;
type ButtonPin = esp_idf_hal::gpio::Gpio13;

#[derive(Debug, Error)]
pub enum Esp32Error {
    #[error("esp-idf error: {0}")]
    Esp(#[from] esp_idf_sys::EspError),
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
    #[error("display error: {0}")]
    Display(#[from] ssd1306::prelude::DisplayError),
}

pub fn run() -> Result<(), Esp32Error> {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take().ok_or_else(|| {
        Esp32Error::Esp(esp_idf_sys::EspError::from_infallible::<
            esp_idf_sys::ESP_ERR_INVALID_STATE,
        >())
    })?;
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

    let mut wifi = BlockingWifi::wrap(EspWifi::new(modem, sysloop.clone(), Some(nvs.clone()))?, sysloop.clone())?;

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
    mdns.add_service(HOSTNAME, "_winderoo", "_tcp", 80, &[])?;

    sync_time()?;

    let _server = start_http_server(shared.clone(), hardware.clone(), storage.clone())?;

    #[cfg(feature = "home-assistant")]
    let ha = HomeAssistant::try_new(shared.clone(), storage.clone())?;

    resume_if_needed(&shared, &hardware, &storage)?;

    run_loop(shared, hardware, storage, wifi, nvs.clone(), #[cfg(feature = "home-assistant")] ha)?;

    Ok(())
}

fn mount_littlefs() -> Result<MountedLittlefs<Littlefs<std::ffi::CString>>, Esp32Error> {
    let littlefs = unsafe { Littlefs::new_partition("littlefs")? };
    let mounted = MountedLittlefs::mount(littlefs, FS_ROOT)?;
    Ok(mounted)
}

fn connect_wifi(wifi: &mut BlockingWifi<EspWifi>, creds: &WifiCredentials) -> Result<(), Esp32Error> {
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

    server.fn_handler("/", Method::Get, move |req| {
        let page = config_portal_page();
        let headers = [content_type("text/html"), cors_allow_origin()];
        let mut response = req.into_response(200, Some("OK"), &headers)?;
        response.write_all(page.as_bytes())?;
        Ok(())
    })?;

    server.fn_handler("/*", Method::Get, move |req| {
        let page = config_portal_page();
        let headers = [content_type("text/html"), cors_allow_origin()];
        let mut response = req.into_response(200, Some("OK"), &headers)?;
        response.write_all(page.as_bytes())?;
        Ok(())
    })?;

    server.fn_handler("/wifi", Method::Post, move |mut req| {
        let mut body = String::new();
        req.read_to_string(&mut body)?;

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
    })?;

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
                    .map(|info| info.rssi)
                    .unwrap_or(-100);
                let time = time_of_day_from_epoch(epoch, guard.controller.state.rtc.gmt_offset, guard.controller.state.rtc.dst);
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
    Ok(guard.button.is_high().unwrap_or(false))
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
    server.fn_handler("/api/status", Method::Get, move |req| {
        let (status, rssi) = {
            let guard = shared_status.lock().map_err(|_| Esp32Error::Lock)?;
            let response = guard
                .controller
                .status_response(current_epoch(), guard.rssi, API_VERSION);
            (response, guard.rssi)
        };
        let body = serde_json::to_string(&status)?;
        respond_json(req, 200, &body)?;
        let _ = rssi;
        Ok(())
    })?;

    let shared_timer = shared.clone();
    let hardware_timer = hardware.clone();
    let storage_timer = storage.clone();
    server.fn_handler("/api/timer", Method::Post, move |req| {
        let timer_enabled = parse_query_bool(req.uri(), "timerEnabled").unwrap_or(false);
        let events = {
            let mut guard = shared_timer.lock().map_err(|_| Esp32Error::Lock)?;
            guard.controller.apply_timer_enabled(timer_enabled)
        };
        apply_events(&shared_timer, &hardware_timer, &storage_timer, events)?;
        respond_empty(req, 204)?;
        Ok(())
    })?;

    let shared_power = shared.clone();
    let hardware_power = hardware.clone();
    let storage_power = storage.clone();
    server.fn_handler("/api/power", Method::Post, move |mut req| {
        let mut body = String::new();
        req.read_to_string(&mut body)?;
        let payload: PowerPayload = serde_json::from_str(&body)?;
        let events = {
            let mut guard = shared_power.lock().map_err(|_| Esp32Error::Lock)?;
            guard.controller.apply_power(payload.winder_enabled)
        };
        apply_events(&shared_power, &hardware_power, &storage_power, events)?;
        respond_empty(req, 204)?;
        Ok(())
    })?;

    let shared_update = shared.clone();
    let hardware_update = hardware.clone();
    let storage_update = storage.clone();
    server.fn_handler("/api/update", Method::Post, move |mut req| {
        let mut body = String::new();
        req.read_to_string(&mut body)?;
        let payload: UpdatePayload = serde_json::from_str(&body)?;
        let update: UpdateRequest = payload.try_into()?;
        let events = {
            let mut guard = shared_update.lock().map_err(|_| Esp32Error::Lock)?;
            guard.controller.apply_update(update, current_epoch())
        };
        apply_events(&shared_update, &hardware_update, &storage_update, events)?;
        respond_empty(req, 204)?;
        Ok(())
    })?;

    let shared_reset = shared.clone();
    let hardware_reset = hardware.clone();
    let storage_reset = storage.clone();
    server.fn_handler("/api/reset", Method::Get, move |req| {
        let response = ResetResponse::new();
        let body = serde_json::to_string(&response)?;
        respond_json(req, 200, &body)?;
        let events = {
            let mut guard = shared_reset.lock().map_err(|_| Esp32Error::Lock)?;
            guard.controller.request_reset()
        };
        apply_events(&shared_reset, &hardware_reset, &storage_reset, events)?;
        Ok(())
    })?;

    let static_storage = storage.clone();
    server.fn_handler("/*", Method::Get, move |req| {
        serve_static(req, &static_storage)
    })?;

    let static_storage_options = storage.clone();
    server.fn_handler("/*", Method::Options, move |req| {
        let _ = static_storage_options;
        respond_empty(req, 200)?;
        Ok(())
    })?;

    Ok(server)
}

fn respond_json<C>(
    req: embedded_svc::http::server::Request<C>,
    status: u16,
    body: &str,
) -> Result<(), Esp32Error>
where
    C: embedded_svc::http::server::Connection,
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
{
    let headers = [cors_allow_origin(), cors_allow_methods(), cors_allow_headers()];
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
        Self { root, settings_path }
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
        let mut ledc = ledc;
        let _i2c0 = i2c0;

        let led_timer = LedcTimerDriver::new(
            ledc.timer0,
            &TimerConfig::default().frequency(5.kHz().into()),
        )?;
        let led_driver = LedcDriver::new(ledc.channel0, &led_timer, pins.gpio2)?;
        let led = LedControl::new(led_timer, led_driver);

        let motor = MotorControl::new(
            pins.gpio25,
            pins.gpio26,
            #[cfg(feature = "pwm-motor")]
            &mut ledc,
        )?;

        let mut button = PinDriver::input(pins.gpio13)?;
        button.set_pull(Pull::Down)?;

        #[cfg(feature = "oled")]
        let display = {
            let config = I2cConfig::new().baudrate(400.kHz().into());
            let i2c = I2cDriver::new(
                _i2c0,
                pins.gpio21,
                pins.gpio22,
                &config,
            )?;
            Some(OledDisplay::new(i2c)?)
        };

        #[cfg(not(feature = "oled"))]
        let display = None;

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
        #[cfg(feature = "pwm-motor")] ledc: &mut esp_idf_hal::ledc::LEDC,
    ) -> Result<Self, Esp32Error> {
        #[cfg(not(feature = "pwm-motor"))]
        let pin_a = PinDriver::output(pin_a)?;
        #[cfg(not(feature = "pwm-motor"))]
        let pin_b = PinDriver::output(pin_b)?;

        #[cfg(feature = "pwm-motor")]
        let pwm = MotorPwm::new(ledc, pin_a, pin_b)?;

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
        ledc: &mut esp_idf_hal::ledc::LEDC,
        pin_a: MotorPinA,
        pin_b: MotorPinB,
    ) -> Result<Self, Esp32Error> {
        let timer = LedcTimerDriver::new(
            ledc.timer1,
            &TimerConfig::default().frequency(1.kHz().into()),
        )?;
        let driver_a = LedcDriver::new(ledc.channel1, &timer, pin_a)?;
        let driver_b = LedcDriver::new(ledc.channel2, &timer, pin_b)?;
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

#[cfg(feature = "oled")]
struct OledDisplay {
    display: Ssd1306<I2CInterface<I2cDriver<'static>>, DisplaySize128x64, ssd1306::mode::BufferedGraphicsMode<DisplaySize128x64>>,
}

#[cfg(feature = "oled")]
impl OledDisplay {
    fn new(i2c: I2cDriver<'static>) -> Result<Self, Esp32Error> {
        let interface = I2CDisplayInterface::new_custom_address(i2c, OLED_ADDR);
        let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0).into_buffered_graphics_mode();
        display.init()?;
        display.set_invert(false)?;
        display.flush()?;
        Ok(Self { display })
    }

    fn clear(&mut self) -> Result<(), Esp32Error> {
        self.display.clear(BinaryColor::Off)?;
        self.display.flush()?;
        Ok(())
    }

    fn draw_static(&mut self, title: &str) -> Result<(), Esp32Error> {
        self.display.clear(BinaryColor::Off)?;

        let text_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();

        Text::with_alignment(title, Point::new(64, 3), text_style, Alignment::Center)
            .draw(&mut self.display)?;

        Line::new(Point::new(0, 14), Point::new(127, 14))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display)?;
        Line::new(Point::new(64, 14), Point::new(64, 50))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display)?;
        Line::new(Point::new(0, 50), Point::new(127, 50))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display)?;

        Text::new("TPD", Point::new(4, 18), text_style).draw(&mut self.display)?;
        Text::new("DIR", Point::new(71, 18), text_style).draw(&mut self.display)?;

        self.display.flush()?;
        Ok(())
    }

    fn draw_dynamic(&mut self, state: &RuntimeState, rssi: i32) -> Result<(), Esp32Error> {
        let small_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();
        let large_style = MonoTextStyleBuilder::new()
            .font(&FONT_10X20)
            .text_color(BinaryColor::On)
            .build();

        Rectangle::new(Point::new(8, 25), Size::new(54, 25))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut self.display)?;
        Text::new(&state.rotations_per_day.to_string(), Point::new(8, 30), large_style)
            .draw(&mut self.display)?;

        Rectangle::new(Point::new(66, 25), Size::new(62, 25))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut self.display)?;
        Text::new(state.direction.as_api_str(), Point::new(74, 30), large_style)
            .draw(&mut self.display)?;

        self.draw_progress_bar(state.cycle_progress)?;
        self.draw_wifi_status(rssi, small_style)?;
        self.draw_timer_status(state, small_style)?;

        self.display.flush()?;
        Ok(())
    }

    fn draw_progress_bar(&mut self, progress: f32) -> Result<(), Esp32Error> {
        Rectangle::new(Point::new(0, 50), Size::new(128, 2))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut self.display)?;
        let width = (progress.clamp(0.0, 1.0) * 128.0) as u32;
        Rectangle::new(Point::new(0, 50), Size::new(width, 2))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut self.display)?;
        Ok(())
    }

    fn draw_wifi_status(
        &mut self,
        rssi: i32,
        style: embedded_graphics::mono_font::MonoTextStyle<'_, BinaryColor>,
    ) -> Result<(), Esp32Error> {
        Triangle::new(Point::new(4, 54), Point::new(10, 54), Point::new(7, 58))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display)?;
        Line::new(Point::new(7, 58), Point::new(7, 62))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display)?;

        Rectangle::new(Point::new(12, 54), Size::new(58, 10))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut self.display)?;

        let bars = if rssi > -50 {
            4
        } else if rssi > -60 {
            3
        } else if rssi > -70 {
            2
        } else {
            1
        };

        for i in 0..bars {
            let height = 2 + (i as i32) * 2;
            Rectangle::new(
                Point::new(14 + i * 4, 55 + (8 - height) as i32),
                Size::new(2, height as u32),
            )
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut self.display)?;
        }

        let _ = style;
        Ok(())
    }

    fn draw_timer_status(
        &mut self,
        state: &RuntimeState,
        style: embedded_graphics::mono_font::MonoTextStyle<'_, BinaryColor>,
    ) -> Result<(), Esp32Error> {
        Rectangle::new(Point::new(60, 54), Size::new(68, 13))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut self.display)?;

        if state.timer.enabled {
            let text = format!("TIMER {:02}:{:02}", state.timer.start_time.hour, state.timer.start_time.minute);
            Text::new(&text, Point::new(60, 56), style).draw(&mut self.display)?;
        }
        Ok(())
    }

    fn draw_notification(&mut self, message: &str) -> Result<(), Esp32Error> {
        Rectangle::new(Point::new(0, 0), Size::new(128, 14))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut self.display)?;

        let text_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::Off)
            .build();

        Text::with_alignment(message, Point::new(64, 3), text_style, Alignment::Center)
            .draw(&mut self.display)?;

        self.display.flush()?;
        thread::sleep(Duration::from_millis(200));

        Rectangle::new(Point::new(0, 0), Size::new(128, 14))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut self.display)?;
        let text_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();
        Text::with_alignment(message, Point::new(64, 3), text_style, Alignment::Center)
            .draw(&mut self.display)?;
        Line::new(Point::new(0, 14), Point::new(127, 14))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display)?;
        self.display.flush()?;
        Ok(())
    }
}

#[cfg(feature = "home-assistant")]
struct HomeAssistant {
    client: EspMqttClient<'static>,
    command_rx: mpsc::Receiver<HaCommand>,
    device_id: String,
}

#[cfg(feature = "home-assistant")]
impl HomeAssistant {
    fn try_new(shared: Arc<Mutex<SharedState>>, _storage: Arc<Storage>) -> Result<Option<Self>, Esp32Error> {
        let broker = env_or_build("HOME_ASSISTANT_BROKER", HA_BROKER_ENV).unwrap_or_default();
        if broker.is_empty() {
            warn!("HOME_ASSISTANT_BROKER not set; skipping MQTT");
            return Ok(None);
        }

        let username = env_or_build("HOME_ASSISTANT_USERNAME", HA_USERNAME_ENV);
        let password = env_or_build("HOME_ASSISTANT_PASSWORD", HA_PASSWORD_ENV);
        let device_id = format!("winderoo-{}", mac_suffix());

        let (command_tx, command_rx) = mpsc::channel();
        let mut config = MqttClientConfiguration::default();
        config.client_id = Some("winderoo");
        config.username = username.as_deref();
        config.password = password.as_deref();

        let url = format!("mqtt://{}", broker);
        let tx = command_tx.clone();
        let client = EspMqttClient::new_cb(&url, &config, move |event| {
            if let EventPayload::Received { topic: Some(topic), data, .. } = event.payload() {
                if let Some(command) = HaCommand::parse(topic, data) {
                    let _ = tx.send(command);
                }
            }
        })?;

        let mut ha = Self {
            client,
            command_rx,
            device_id: device_id.clone(),
        };

        ha.publish_config(&shared, &storage)?;
        ha.subscribe_commands()?;

        Ok(Some(ha))
    }

    fn subscribe_commands(&mut self) -> Result<(), Esp32Error> {
        let topics = HaCommand::topics(&self.device_id);
        for topic in topics {
            let _ = self.client.subscribe(&topic, QoS::AtMostOnce);
        }
        Ok(())
    }

    fn publish_config(
        &mut self,
        shared: &Arc<Mutex<SharedState>>,
        _storage: &Arc<Storage>,
    ) -> Result<(), Esp32Error> {
        let guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
        let config_messages = ha_config_messages(&self.device_id, &guard.controller.state);
        for (topic, payload) in config_messages {
            let _ = self.client.publish(&topic, QoS::AtMostOnce, true, payload.as_bytes());
        }
        Ok(())
    }

    fn publish_state(&mut self, shared: &Arc<Mutex<SharedState>>) -> Result<(), Esp32Error> {
        let guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
        let state_messages = ha_state_messages(&self.device_id, &guard.controller.state, guard.rssi);
        for (topic, payload) in state_messages {
            let _ = self.client.publish(&topic, QoS::AtMostOnce, false, payload.as_bytes());
        }
        Ok(())
    }

    fn drain_commands(
        &mut self,
        shared: &Arc<Mutex<SharedState>>,
        hardware: &Arc<Mutex<Hardware>>,
        storage: &Arc<Storage>,
    ) -> Result<(), Esp32Error> {
        while let Ok(cmd) = self.command_rx.try_recv() {
            let events = {
                let mut guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
                cmd.apply(&mut guard.controller)
            };
            apply_events(shared, hardware, storage, events)?;
        }
        Ok(())
    }
}

#[cfg(feature = "home-assistant")]
fn env_or_build(key: &str, build_value: Option<&'static str>) -> Option<String> {
    std::env::var(key).ok().or_else(|| build_value.map(|value| value.to_string()))
}

#[cfg(feature = "home-assistant")]
#[derive(Debug, Clone)]
enum HaCommand {
    Power(bool),
    Timer(bool),
    Oled(bool),
    Start,
    Stop,
    Direction(Direction),
    Rpd(u16),
    Hour(u8),
    Minute(u8),
    CustomWindDuration(u32),
    CustomWindPause(u32),
    RotationDuration(u16),
    RtcOffset(f32),
    RtcDst(bool),
    ScreenScheduleEnabled(bool),
    ScreenScheduleStartHour(u8),
    ScreenScheduleStartMinute(u8),
    ScreenScheduleEndHour(u8),
    ScreenScheduleEndMinute(u8),
}

#[cfg(feature = "home-assistant")]
impl HaCommand {
    fn topics(device_id: &str) -> Vec<String> {
        vec![
            format!("winderoo/{device_id}/power/set"),
            format!("winderoo/{device_id}/timer/set"),
            format!("winderoo/{device_id}/oled/set"),
            format!("winderoo/{device_id}/start/set"),
            format!("winderoo/{device_id}/stop/set"),
            format!("winderoo/{device_id}/direction/set"),
            format!("winderoo/{device_id}/rpd/set"),
            format!("winderoo/{device_id}/hour/set"),
            format!("winderoo/{device_id}/minute/set"),
            format!("winderoo/{device_id}/custom_wind_duration/set"),
            format!("winderoo/{device_id}/custom_wind_pause/set"),
            format!("winderoo/{device_id}/rotation_duration/set"),
            format!("winderoo/{device_id}/rtc_offset/set"),
            format!("winderoo/{device_id}/rtc_dst/set"),
            format!("winderoo/{device_id}/screen_schedule_enabled/set"),
            format!("winderoo/{device_id}/screen_schedule_start_hour/set"),
            format!("winderoo/{device_id}/screen_schedule_start_minute/set"),
            format!("winderoo/{device_id}/screen_schedule_end_hour/set"),
            format!("winderoo/{device_id}/screen_schedule_end_minute/set"),
        ]
    }

    fn parse(topic: &str, payload: &[u8]) -> Option<Self> {
        let payload = core::str::from_utf8(payload).ok()?.trim();
        if topic.ends_with("/power/set") {
            return Some(Self::Power(parse_bool(payload)));
        }
        if topic.ends_with("/timer/set") {
            return Some(Self::Timer(parse_bool(payload)));
        }
        if topic.ends_with("/oled/set") {
            return Some(Self::Oled(parse_bool(payload)));
        }
        if topic.ends_with("/start/set") {
            return Some(Self::Start);
        }
        if topic.ends_with("/stop/set") {
            return Some(Self::Stop);
        }
        if topic.ends_with("/direction/set") {
            let direction = Direction::from_api(payload)?;
            return Some(Self::Direction(direction));
        }
        if topic.ends_with("/rpd/set") {
            return payload.parse().ok().map(Self::Rpd);
        }
        if topic.ends_with("/hour/set") {
            return payload.parse().ok().map(Self::Hour);
        }
        if topic.ends_with("/minute/set") {
            return payload.parse().ok().map(Self::Minute);
        }
        if topic.ends_with("/custom_wind_duration/set") {
            return payload.parse().ok().map(Self::CustomWindDuration);
        }
        if topic.ends_with("/custom_wind_pause/set") {
            return payload.parse().ok().map(Self::CustomWindPause);
        }
        if topic.ends_with("/rotation_duration/set") {
            return payload.parse().ok().map(Self::RotationDuration);
        }
        if topic.ends_with("/rtc_offset/set") {
            return payload.parse().ok().map(Self::RtcOffset);
        }
        if topic.ends_with("/rtc_dst/set") {
            return Some(Self::RtcDst(parse_bool(payload)));
        }
        if topic.ends_with("/screen_schedule_enabled/set") {
            return Some(Self::ScreenScheduleEnabled(parse_bool(payload)));
        }
        if topic.ends_with("/screen_schedule_start_hour/set") {
            return payload.parse().ok().map(Self::ScreenScheduleStartHour);
        }
        if topic.ends_with("/screen_schedule_start_minute/set") {
            return payload.parse().ok().map(Self::ScreenScheduleStartMinute);
        }
        if topic.ends_with("/screen_schedule_end_hour/set") {
            return payload.parse().ok().map(Self::ScreenScheduleEndHour);
        }
        if topic.ends_with("/screen_schedule_end_minute/set") {
            return payload.parse().ok().map(Self::ScreenScheduleEndMinute);
        }
        None
    }

    fn apply(self, controller: &mut Controller<XorShift32>) -> Vec<ControllerEvent> {
        match self {
            HaCommand::Power(enabled) => controller.apply_power(enabled),
            HaCommand::Timer(enabled) => controller.apply_timer_enabled(enabled),
            HaCommand::Oled(enabled) => {
                controller.state.screen.sleep = !enabled;
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
            HaCommand::Start => {
                let now = current_epoch();
                if !controller.state.routine.running {
                    controller.apply_update(build_update(controller, UpdateAction::Start), now)
                } else {
                    Vec::new()
                }
            }
            HaCommand::Stop => {
                let now = current_epoch();
                controller.apply_update(build_update(controller, UpdateAction::Stop), now)
            }
            HaCommand::Direction(direction) => {
                controller.state.direction = direction;
                controller.state.motor_direction = match direction {
                    Direction::Clockwise => MotorDirection::Clockwise,
                    Direction::CounterClockwise => MotorDirection::CounterClockwise,
                    Direction::Both => controller.state.motor_direction,
                };
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
            HaCommand::Rpd(rpd) => {
                controller.state.rotations_per_day = rpd;
                if controller.state.routine.running {
                    controller.state.routine.estimated_finish_epoch =
                        current_epoch() + calculate_winding_duration_secs(&controller.state);
                }
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
            HaCommand::Hour(hour) => {
                if let Ok(time) = TimeOfDay::new(hour, controller.state.timer.start_time.minute) {
                    controller.state.timer.start_time = time;
                }
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
            HaCommand::Minute(minute) => {
                if let Ok(time) = TimeOfDay::new(controller.state.timer.start_time.hour, minute) {
                    controller.state.timer.start_time = time;
                }
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
            HaCommand::CustomWindDuration(duration) => {
                controller.state.custom_wind_duration_secs = duration;
                if controller.state.routine.running {
                    controller.state.routine.estimated_finish_epoch =
                        current_epoch() + calculate_winding_duration_secs(&controller.state);
                }
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
            HaCommand::CustomWindPause(duration) => {
                controller.state.custom_wind_pause_secs = duration;
                if controller.state.routine.running {
                    controller.state.routine.estimated_finish_epoch =
                        current_epoch() + calculate_winding_duration_secs(&controller.state);
                }
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
            HaCommand::RotationDuration(duration) => {
                controller.state.rotation_duration_secs = duration;
                if controller.state.routine.running {
                    controller.state.routine.estimated_finish_epoch =
                        current_epoch() + calculate_winding_duration_secs(&controller.state);
                }
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
            HaCommand::RtcOffset(offset) => {
                controller.state.rtc.gmt_offset = offset;
                vec![
                    ControllerEvent::SyncTime,
                    ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state)),
                ]
            }
            HaCommand::RtcDst(dst) => {
                controller.state.rtc.dst = dst;
                vec![
                    ControllerEvent::SyncTime,
                    ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state)),
                ]
            }
            HaCommand::ScreenScheduleEnabled(enabled) => {
                controller.state.screen.schedule.enabled = enabled;
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
            HaCommand::ScreenScheduleStartHour(hour) => {
                if let Ok(time) = TimeOfDay::new(hour, controller.state.screen.schedule.start.minute) {
                    controller.state.screen.schedule.start = time;
                }
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
            HaCommand::ScreenScheduleStartMinute(minute) => {
                if let Ok(time) = TimeOfDay::new(controller.state.screen.schedule.start.hour, minute) {
                    controller.state.screen.schedule.start = time;
                }
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
            HaCommand::ScreenScheduleEndHour(hour) => {
                if let Ok(time) = TimeOfDay::new(hour, controller.state.screen.schedule.end.minute) {
                    controller.state.screen.schedule.end = time;
                }
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
            HaCommand::ScreenScheduleEndMinute(minute) => {
                if let Ok(time) = TimeOfDay::new(controller.state.screen.schedule.end.hour, minute) {
                    controller.state.screen.schedule.end = time;
                }
                vec![ControllerEvent::PersistSettings(StoredSettings::from_runtime(&controller.state))]
            }
        }
    }
}

#[cfg(feature = "home-assistant")]
fn build_update(controller: &Controller<XorShift32>, action: UpdateAction) -> UpdateRequest {
    UpdateRequest {
        direction: controller.state.direction,
        rotations_per_day: controller.state.rotations_per_day,
        action,
        hour: controller.state.timer.start_time.hour,
        minutes: controller.state.timer.start_time.minute,
        timer_enabled: controller.state.timer.enabled,
        screen_sleep: controller.state.screen.sleep,
        screen_schedule_enabled: Some(controller.state.screen.schedule.enabled),
        screen_schedule_start: Some(controller.state.screen.schedule.start),
        screen_schedule_end: Some(controller.state.screen.schedule.end),
        custom_wind_duration_secs: controller.state.custom_wind_duration_secs,
        custom_wind_pause_secs: controller.state.custom_wind_pause_secs,
        rotation_duration_secs: controller.state.rotation_duration_secs,
        rtc_gmt_offset: controller.state.rtc.gmt_offset,
        rtc_dst: controller.state.rtc.dst,
    }
}

#[cfg(feature = "home-assistant")]
fn parse_bool(value: &str) -> bool {
    matches!(value, "1" | "true" | "True" | "TRUE" | "on" | "ON")
}

#[cfg(feature = "home-assistant")]
fn mac_suffix() -> String {
    unsafe {
        let mut mac = [0u8; 6];
        esp_idf_sys::esp_read_mac(mac.as_mut_ptr(), esp_idf_sys::esp_mac_type_t_ESP_MAC_WIFI_STA);
        format!("{:02X}{:02X}{:02X}", mac[3], mac[4], mac[5])
    }
}

#[cfg(feature = "home-assistant")]
fn ha_config_messages(device_id: &str, _state: &RuntimeState) -> Vec<(String, String)> {
    let base = format!("winderoo/{device_id}");
    let device = format!("{{\"identifiers\":[\"{device_id}\"],\"name\":\"Winderoo\",\"model\":\"Winderoo\",\"manufacturer\":\"mwood77\",\"sw_version\":\"{API_VERSION}\"}}");
    vec![
        (
            format!("homeassistant/switch/{device_id}_power/config"),
            format!("{{\"name\":\"Power\",\"state_topic\":\"{base}/power\",\"command_topic\":\"{base}/power/set\",\"icon\":\"mdi:power\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/switch/{device_id}_timer/config"),
            format!("{{\"name\":\"Timer Enabled\",\"state_topic\":\"{base}/timer\",\"command_topic\":\"{base}/timer/set\",\"icon\":\"mdi:timer\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/switch/{device_id}_oled/config"),
            format!("{{\"name\":\"OLED\",\"state_topic\":\"{base}/oled\",\"command_topic\":\"{base}/oled/set\",\"icon\":\"mdi:overscan\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/button/{device_id}_start/config"),
            format!("{{\"name\":\"Start\",\"command_topic\":\"{base}/start/set\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/button/{device_id}_stop/config"),
            format!("{{\"name\":\"Stop\",\"command_topic\":\"{base}/stop/set\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/number/{device_id}_rpd/config"),
            format!("{{\"name\":\"Rotations Per Day\",\"state_topic\":\"{base}/rpd\",\"command_topic\":\"{base}/rpd/set\",\"min\":100,\"max\":960,\"step\":10,\"icon\":\"mdi:rotate-3d-variant\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_direction/config"),
            format!("{{\"name\":\"Direction\",\"state_topic\":\"{base}/direction\",\"command_topic\":\"{base}/direction/set\",\"options\":[\"CCW\",\"BOTH\",\"CW\"],\"icon\":\"mdi:arrow-left-right\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_hour/config"),
            format!("{{\"name\":\"Hour\",\"state_topic\":\"{base}/hour\",\"command_topic\":\"{base}/hour/set\",\"options\":[\"00\",\"01\",\"02\",\"03\",\"04\",\"05\",\"06\",\"07\",\"08\",\"09\",\"10\",\"11\",\"12\",\"13\",\"14\",\"15\",\"16\",\"17\",\"18\",\"19\",\"20\",\"21\",\"22\",\"23\"],\"icon\":\"mdi:timer-sand-full\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_minute/config"),
            format!("{{\"name\":\"Minutes\",\"state_topic\":\"{base}/minute\",\"command_topic\":\"{base}/minute/set\",\"options\":[\"00\",\"10\",\"20\",\"30\",\"40\",\"50\"],\"icon\":\"mdi:timer-sand-empty\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/sensor/{device_id}_status/config"),
            format!("{{\"name\":\"Status\",\"state_topic\":\"{base}/status\",\"icon\":\"mdi:information\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/sensor/{device_id}_rssi/config"),
            format!("{{\"name\":\"WiFi RSSI\",\"state_topic\":\"{base}/rssi\",\"unit_of_measurement\":\"dBm\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/number/{device_id}_custom_wind_duration/config"),
            format!("{{\"name\":\"Time to Rotate\",\"state_topic\":\"{base}/custom_wind_duration\",\"command_topic\":\"{base}/custom_wind_duration/set\",\"min\":100,\"max\":960,\"step\":10,\"icon\":\"mdi:play-circle-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/number/{device_id}_custom_wind_pause/config"),
            format!("{{\"name\":\"Time to Pause\",\"state_topic\":\"{base}/custom_wind_pause\",\"command_topic\":\"{base}/custom_wind_pause/set\",\"min\":10,\"max\":900,\"step\":5,\"icon\":\"mdi:pause-circle-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/number/{device_id}_rotation_duration/config"),
            format!("{{\"name\":\"Rotation Duration\",\"state_topic\":\"{base}/rotation_duration\",\"command_topic\":\"{base}/rotation_duration/set\",\"min\":1,\"max\":16,\"step\":1,\"icon\":\"mdi:arrow-u-down-right\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_rtc_offset/config"),
            format!("{{\"name\":\"UTC Offset\",\"state_topic\":\"{base}/rtc_offset\",\"command_topic\":\"{base}/rtc_offset/set\",\"options\":[\"-12\",\"-11\",\"-10\",\"-9.5\",\"-9\",\"-8\",\"-7\",\"-6\",\"-5\",\"-4.5\",\"-4\",\"-3.5\",\"-3\",\"-2\",\"-1\",\"0\",\"1\",\"2\",\"3\",\"3.5\",\"4\",\"4.5\",\"5\",\"5.5\",\"5.75\",\"6\",\"6.5\",\"7\",\"8\",\"8.75\",\"9\",\"9.5\",\"10\",\"10.5\",\"11\",\"11.5\",\"12\",\"12.75\",\"13\",\"14\"],\"icon\":\"mdi:clock-time-eight-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/switch/{device_id}_rtc_dst/config"),
            format!("{{\"name\":\"DST\",\"state_topic\":\"{base}/rtc_dst\",\"command_topic\":\"{base}/rtc_dst/set\",\"icon\":\"mdi:clock-time-four-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/switch/{device_id}_screen_schedule/config"),
            format!("{{\"name\":\"Screen Schedule Enabled\",\"state_topic\":\"{base}/screen_schedule_enabled\",\"command_topic\":\"{base}/screen_schedule_enabled/set\",\"icon\":\"mdi:calendar-clock\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_screen_schedule_start_hour/config"),
            format!("{{\"name\":\"Screen Schedule Start Hour\",\"state_topic\":\"{base}/screen_schedule_start_hour\",\"command_topic\":\"{base}/screen_schedule_start_hour/set\",\"options\":[\"00\",\"01\",\"02\",\"03\",\"04\",\"05\",\"06\",\"07\",\"08\",\"09\",\"10\",\"11\",\"12\",\"13\",\"14\",\"15\",\"16\",\"17\",\"18\",\"19\",\"20\",\"21\",\"22\",\"23\"],\"icon\":\"mdi:clock-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_screen_schedule_start_minute/config"),
            format!("{{\"name\":\"Screen Schedule Start Minute\",\"state_topic\":\"{base}/screen_schedule_start_minute\",\"command_topic\":\"{base}/screen_schedule_start_minute/set\",\"options\":[\"00\",\"10\",\"20\",\"30\",\"40\",\"50\"],\"icon\":\"mdi:clock-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_screen_schedule_end_hour/config"),
            format!("{{\"name\":\"Screen Schedule End Hour\",\"state_topic\":\"{base}/screen_schedule_end_hour\",\"command_topic\":\"{base}/screen_schedule_end_hour/set\",\"options\":[\"00\",\"01\",\"02\",\"03\",\"04\",\"05\",\"06\",\"07\",\"08\",\"09\",\"10\",\"11\",\"12\",\"13\",\"14\",\"15\",\"16\",\"17\",\"18\",\"19\",\"20\",\"21\",\"22\",\"23\"],\"icon\":\"mdi:clock-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_screen_schedule_end_minute/config"),
            format!("{{\"name\":\"Screen Schedule End Minute\",\"state_topic\":\"{base}/screen_schedule_end_minute\",\"command_topic\":\"{base}/screen_schedule_end_minute/set\",\"options\":[\"00\",\"10\",\"20\",\"30\",\"40\",\"50\"],\"icon\":\"mdi:clock-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/sensor/{device_id}_rtc_epoch/config"),
            format!("{{\"name\":\"RTC Epoch Time\",\"state_topic\":\"{base}/rtc_epoch\",\"icon\":\"mdi:clock-time-nine-outline\",\"device\":{device}}}"),
        ),
    ]
}

#[cfg(feature = "home-assistant")]
fn ha_state_messages(device_id: &str, state: &RuntimeState, rssi: i32) -> Vec<(String, String)> {
    let base = format!("winderoo/{device_id}");
    vec![
        (format!("{base}/power"), if state.winder_enabled { "ON" } else { "OFF" }.to_string()),
        (format!("{base}/timer"), if state.timer.enabled { "ON" } else { "OFF" }.to_string()),
        (format!("{base}/oled"), if state.screen.sleep { "OFF" } else { "ON" }.to_string()),
        (format!("{base}/status"), state.status_str().to_string()),
        (format!("{base}/rssi"), rssi.to_string()),
        (format!("{base}/direction"), state.direction.as_api_str().to_string()),
        (format!("{base}/rpd"), state.rotations_per_day.to_string()),
        (format!("{base}/hour"), format!("{:02}", state.timer.start_time.hour)),
        (format!("{base}/minute"), format!("{:02}", state.timer.start_time.minute)),
        (format!("{base}/custom_wind_duration"), state.custom_wind_duration_secs.to_string()),
        (format!("{base}/custom_wind_pause"), state.custom_wind_pause_secs.to_string()),
        (format!("{base}/rotation_duration"), state.rotation_duration_secs.to_string()),
        (format!("{base}/rtc_offset"), state.rtc.gmt_offset.to_string()),
        (format!("{base}/rtc_dst"), if state.rtc.dst { "ON" } else { "OFF" }.to_string()),
        (format!("{base}/screen_schedule_enabled"), if state.screen.schedule.enabled { "ON" } else { "OFF" }.to_string()),
        (format!("{base}/screen_schedule_start_hour"), format!("{:02}", state.screen.schedule.start.hour)),
        (format!("{base}/screen_schedule_start_minute"), format!("{:02}", state.screen.schedule.start.minute)),
        (format!("{base}/screen_schedule_end_hour"), format!("{:02}", state.screen.schedule.end.hour)),
        (format!("{base}/screen_schedule_end_minute"), format!("{:02}", state.screen.schedule.end.minute)),
        (format!("{base}/rtc_epoch"), current_epoch().to_string()),
    ]
}

#[cfg(feature = "home-assistant")]
use crate::api::UpdateAction;
