//! ESP32 bridge from controller events to hardware side-effects.
//!
//! The controller emits a stream of [`ControllerEvent`]s. This module applies them to:
//! - Hardware (motor/LED/OLED)
//! - Persistent settings (LittleFS)
//! - Runtime state flags (restart request)

use crate::controller::ControllerEvent;
use log::warn;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::{Esp32Error, Hardware, SharedState, Storage};

pub(super) fn apply_events(
    shared: &Arc<Mutex<SharedState>>,
    hardware: &Arc<Mutex<Hardware>>,
    storage: &Arc<Storage>,
    events: Vec<ControllerEvent>,
) -> Result<(), Esp32Error> {
    if events.is_empty() {
        return Ok(());
    }

    // Serialize *all* hardware side-effects behind a single lock.
    // This keeps motor/LED/OLED updates ordered and avoids concurrent access to GPIO/I2C/LEDC.
    //
    // Important lock rule: callers should NOT hold the `shared` mutex when calling `apply_events`.
    // Some events (like `DisplayDynamic`) need a quick read from shared state while we already
    // hold the hardware lock, and holding both in opposite order is a classic deadlock footgun.
    let mut hw = hardware.lock().map_err(|_| Esp32Error::Lock)?;

    for event in events {
        match event {
            ControllerEvent::MotorStart(dir) => hw.motor_start(dir)?,
            ControllerEvent::MotorStop => hw.motor_stop()?,
            ControllerEvent::PauseSeconds(secs) => {
                // Ported from Arduino-style `delay()` usage:
                // the controller schedules real time pauses to shape motor duty cycles.
                // This blocks the *event applier* thread (usually the main loop).
                thread::sleep(Duration::from_secs(secs as u64));
            }
            ControllerEvent::DisplayClear => hw.display_clear()?,
            ControllerEvent::DisplayStatic { title } => {
                hw.display_static(&title)?;
            }
            ControllerEvent::DisplayDynamic => {
                // Dynamic OLED data is derived from controller state + current RSSI.
                // We snapshot under the shared lock so the display render is consistent.
                let guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
                hw.display_dynamic(&guard.controller.state, guard.rssi)?;
            }
            ControllerEvent::DisplayNotification(message) => {
                hw.display_notification(&message)?;
            }
            ControllerEvent::Led(pattern) => hw.led_trigger(pattern)?,
            ControllerEvent::PersistSettings(settings) => {
                // C++ wrote `settings.json` directly from endpoints / MQTT callbacks.
                // Rust keeps that decision in the controller, and executes persistence here.
                storage.save(&settings)?
            }
            ControllerEvent::SyncTime => {
                if let Err(err) = super::sync_time() {
                    warn!("time sync failed: {err}");
                }
            }
            ControllerEvent::RestartDevice => {
                // Avoid restarting from random threads/handlers.
                // We raise a flag and let the main loop perform the reset after flushing storage
                // and showing UI feedback (mirrors the C++ `reset` flag behavior).
                let mut guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
                guard.reset_requested = true;
            }
        }
    }

    Ok(())
}

