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
                if let Err(err) = super::sync_time() {
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

