//! ESP32 HTTP server (API + static assets).
//!
//! This module owns the ESP-IDF HTTP server wiring:
//! - REST-ish API endpoints used by the Angular frontend.
//! - Static asset serving from LittleFS (with optional `.gz`).
//! - CORS headers for browser access.

use crate::api::{PowerPayload, ResetResponse, UpdatePayload, UpdateRequest};
use crate::controller::ControllerEvent;
use embedded_svc::http::headers::content_type;
use embedded_svc::http::Method;
use embedded_svc::io::Write as SvcWrite;
use esp_idf_svc::http::server::{Configuration as HttpConfig, EspHttpServer};
use std::sync::{Arc, Mutex};

use super::events::apply_events;
use super::{current_epoch, Esp32Error, Hardware, SharedState, Storage, API_VERSION};

pub(super) fn start_http_server(
    shared: Arc<Mutex<SharedState>>,
    hardware: Arc<Mutex<Hardware>>,
    storage: Arc<Storage>,
) -> Result<EspHttpServer<'static>, Esp32Error> {
    // C++ used AsyncWebServer with global singletons.
    // Here we build an ESP-IDF server and move `Arc` clones into handler closures.
    // Each handler treats the controller as the source of truth and emits events to apply.
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
            let status = {
                // Only a short read-lock: build the response from a consistent snapshot.
                let guard = shared_status.lock().map_err(|_| Esp32Error::Lock)?;
                guard
                    .controller
                    .status_response(current_epoch(), guard.rssi, API_VERSION)
            };
            let body = serde_json::to_string(&status)?;
            respond_json(req, 200, &body)?;
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
            // Matches the legacy firmware: `timerEnabled` is sent as a query param.
            let timer_enabled = parse_query_bool(req.uri(), "timerEnabled").unwrap_or(false);
            let events = {
                // Lock only while mutating controller state.
                let mut guard = shared_timer.lock().map_err(|_| Esp32Error::Lock)?;
                guard.controller.apply_timer_enabled(timer_enabled)
            };
            // Apply side-effects after dropping the controller lock.
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
            // Parse request body outside the lock: JSON parsing can be relatively slow.
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
                // All "commands" become controller mutations that yield a list of side-effects.
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
            // Respond immediately so the client UI doesn't hang waiting for the reset to happen.
            let response = ResetResponse::new();
            let body = serde_json::to_string(&response)?;
            respond_json(req, 200, &body)?;
            let events: Vec<ControllerEvent> = {
                let mut guard = shared_reset.lock().map_err(|_| Esp32Error::Lock)?;
                guard.controller.request_reset()
            };
            apply_events(&shared_reset, &hardware_reset, &storage_reset, events)?;
            Ok(())
        },
    )?;

    let static_storage = storage.clone();
    server.fn_handler("/*", Method::Get, move |req| -> Result<(), Esp32Error> {
        // Serve the bundled web UI from LittleFS (similar to `server.serveStatic(...)` in C++).
        serve_static(req, &static_storage)
    })?;

    let static_storage_options = storage.clone();
    server.fn_handler(
        "/*",
        Method::Options,
        move |req| -> Result<(), Esp32Error> {
            let _ = static_storage_options;
            // Browser CORS preflight (C++ did this with DefaultHeaders + an OPTIONS handler).
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
        // Mirrors `DefaultHeaders::Instance().addHeader(...)` in the Arduino firmware.
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
    // `Storage` encapsulates the "LittleFS + gzip + content-type" logic so the server is dumb.
    let asset = storage.resolve_asset(req.uri());
    match asset {
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

pub(super) fn cors_allow_origin() -> (&'static str, &'static str) {
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

pub(super) fn read_request_body<C>(
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
