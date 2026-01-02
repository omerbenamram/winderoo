//! ESP32 Wi‑Fi connection + AP-mode captive portal.
//!
//! This module owns the ESP-IDF Wi‑Fi drivers and the small HTTP server used as a captive portal.
//! Parsing + HTML lives in `crate::wifi_portal` so it can be unit-tested on a host.

use crate::wifi_portal;
use embedded_svc::http::headers::content_type;
use embedded_svc::http::Method;
use embedded_svc::io::Write as SvcWrite;
use esp_idf_svc::http::server::{Configuration as HttpConfig, EspHttpServer};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs};
use esp_idf_svc::wifi::{
    AccessPointConfiguration, AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi,
};
use heapless::String as HeaplessString;
use log::info;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::{
    cors_allow_origin, read_request_body, Esp32Error, Hardware, SharedState, Storage, AP_SSID,
};

#[derive(Debug, Clone)]
pub(super) struct WifiCredentials {
    pub(super) ssid: String,
    pub(super) password: String,
}

impl WifiCredentials {
    pub(super) fn load(nvs: &EspDefaultNvsPartition) -> Result<Option<Self>, Esp32Error> {
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

    pub(super) fn save(
        nvs: &EspDefaultNvsPartition,
        creds: &WifiCredentials,
    ) -> Result<(), Esp32Error> {
        let mut nvs = EspNvs::new(nvs.clone(), "wifi", true)?;
        nvs.set_str("ssid", &creds.ssid)?;
        nvs.set_str("password", &creds.password)?;
        Ok(())
    }

    pub(super) fn clear(nvs: &EspDefaultNvsPartition) -> Result<(), Esp32Error> {
        let mut nvs = EspNvs::new(nvs.clone(), "wifi", true)?;
        let _ = nvs.remove("ssid");
        let _ = nvs.remove("password");
        Ok(())
    }
}

pub(super) fn connect_wifi(
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

pub(super) fn start_config_portal(
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
        let page = wifi_portal::config_portal_page();
        let headers = [content_type("text/html"), cors_allow_origin()];
        let mut response = req.into_response(200, Some("OK"), &headers)?;
        response.write_all(page.as_bytes())?;
        Ok(())
    })?;

    server.fn_handler("/*", Method::Get, move |req| -> Result<(), Esp32Error> {
        let page = wifi_portal::config_portal_page();
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
            if let Some((ssid, password)) = wifi_portal::parse_wifi_payload(&body) {
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
            super::notify_and_restart(shared, hardware, storage, nvs)?;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn to_heapless<const N: usize>(value: &str) -> Result<HeaplessString<N>, Esp32Error> {
    HeaplessString::try_from(value).map_err(|_| Esp32Error::InvalidConfig(value.to_string()))
}
