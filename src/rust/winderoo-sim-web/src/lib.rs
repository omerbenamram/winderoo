//! WASM bindings for the Winderoo simulator.
//!
//! This module exposes the firmware simulation engine to JavaScript via wasm-bindgen,
//! allowing a web-based Three.js frontend to drive the simulation.
//!
//! The display rendering uses the SAME embedded-graphics code as the real firmware,
//! ensuring pixel-perfect accuracy between simulator and hardware.

mod display;

use display::{DisplaySnapshot, SimDisplay, DISPLAY_HEIGHT, DISPLAY_WIDTH};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use winderoo_firmware::controller::{Controller, ControllerEvent};
use winderoo_firmware::hardware::{LedPattern, XorShift32};
use winderoo_firmware::model::{
    Direction, MotorDirection, RoutineState, RtcConfig, RuntimeState, ScreenSchedule, ScreenState,
    TimerConfig, UpdateAction, UpdateRequest, WinderStatus,
};
use winderoo_firmware::settings::StoredSettings;
use winderoo_firmware::time::{time_of_day_from_epoch, TimeOfDay};

extern crate alloc;
use alloc::{string::String, vec::Vec};

#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// A traced event from the simulation, sent to JS for visualization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TracedEvent {
    MotorStart { direction: String },
    MotorStop,
    PauseStart { seconds: u32 },
    PauseEnd,
    DisplayClear,
    DisplayStatic { title: String },
    DisplayDynamic,
    DisplayNotification { message: String },
    Led { pattern: String },
    PersistSettings,
    SyncTime,
    RestartDevice,
    Tick { epoch: u64 },
}

/// A coalesced trace entry.
///
/// The firmware's controller emits level/heartbeat commands (eg `MotorStart`, `DisplayDynamic`)
/// on every tick while winding / screen-on. Logging those raw would flood the UI.
///
/// We coalesce consecutive identical events into a single entry with a `count`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    #[serde(flatten)]
    pub event: TracedEvent,
    pub count: u32,
}

/// The current state snapshot exposed to JS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimState {
    // Time
    pub now_ms: u64,
    pub now_epoch: u64,
    pub time_of_day: String,

    // Motor
    pub motor_running: bool,
    pub motor_direction: Option<String>,
    pub motor_angle: f64, // For 3D rendering - cumulative rotation angle

    // LED
    pub led_pattern: Option<String>,

    // Display
    pub display_on: bool,
    pub display_title: Option<String>,
    pub display_notification: Option<String>,

    // Runtime state
    pub winder_enabled: bool,
    pub status: String,
    pub direction: String,
    pub rotations_per_day: u16,
    pub timer_enabled: bool,
    pub timer_time: String,
    pub cycle_progress: f32,

    // Routine
    pub routine_running: bool,
    pub routine_start_epoch: u64,
    pub routine_finish_epoch: u64,

    // Config
    pub custom_wind_duration_secs: u32,
    pub custom_wind_pause_secs: u32,
    pub rotation_duration_secs: u16,
    pub gmt_offset: f32,
    pub dst: bool,
    pub screen_schedule_enabled: bool,
    pub screen_schedule_start: String,
    pub screen_schedule_end: String,
    pub screen_sleep: bool,
}

/// The main simulator engine exposed to JavaScript.
#[wasm_bindgen]
pub struct WasmSimulator {
    now_ms: u64,
    tick_interval_ms: u64,
    stored_settings: StoredSettings,
    controller: Controller<XorShift32>,

    // Simulated hardware state
    motor_running: bool,
    motor_direction: Option<MotorDirection>,
    motor_angle: f64,            // cumulative angle for 3D viz
    motor_angular_velocity: f64, // degrees per ms

    led_pattern: Option<LedPattern>,

    /// Simulated OLED display using the SAME rendering code as real firmware.
    sim_display: SimDisplay,
    display_on: bool,

    // Event trace for visualization
    trace: Vec<TraceEntry>,
    max_trace_entries: usize,

    // Pause handling
    pause_remaining_ms: u64,
}

#[wasm_bindgen]
impl WasmSimulator {
    /// Create a new simulator instance.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        let stored_settings = StoredSettings::default();
        let state = stored_settings
            .to_runtime(true)
            .unwrap_or_else(|_| Self::default_runtime_state());
        let rng = XorShift32::new(1);
        let controller = Controller::new(state, rng);

        let mut sim_display = SimDisplay::new();
        sim_display.draw_static("Stopped");

        Self {
            now_ms: 0,
            tick_interval_ms: 500,
            stored_settings,
            controller,
            motor_running: false,
            motor_direction: None,
            motor_angle: 0.0,
            motor_angular_velocity: 45.0, // 45 degrees per second = nice visible rotation
            led_pattern: None,
            sim_display,
            display_on: true,
            trace: Vec::new(),
            max_trace_entries: 500,
            pause_remaining_ms: 0,
        }
    }

    fn default_runtime_state() -> RuntimeState {
        RuntimeState {
            status: WinderStatus::Stopped,
            rotations_per_day: 220,
            direction: Direction::Both,
            motor_direction: MotorDirection::CounterClockwise,
            timer: TimerConfig {
                enabled: false,
                start_time: TimeOfDay::new(0, 0).unwrap(),
            },
            winder_enabled: true,
            custom_wind_duration_secs: 180,
            custom_wind_pause_secs: 15,
            rotation_duration_secs: 8,
            rtc: RtcConfig {
                gmt_offset: 0.0,
                dst: false,
            },
            screen: ScreenState {
                equipped: true,
                sleep: false,
                schedule: ScreenSchedule {
                    enabled: false,
                    start: TimeOfDay::new(0, 0).unwrap(),
                    end: TimeOfDay::new(0, 0).unwrap(),
                },
            },
            routine: RoutineState::idle(),
            cycle_progress: 0.0,
        }
    }

    /// Get the current simulation state as a JS object.
    #[wasm_bindgen(js_name = getState)]
    pub fn get_state(&self) -> JsValue {
        let state = SimState {
            now_ms: self.now_ms,
            now_epoch: self.now_ms / 1000,
            time_of_day: {
                let epoch = self.now_ms / 1000;
                let seconds = (epoch % 86_400) as u32;
                let h = seconds / 3_600;
                let m = (seconds / 60) % 60;
                let s = seconds % 60;
                format!("{:02}:{:02}:{:02}", h, m, s)
            },
            motor_running: self.motor_running,
            motor_direction: self.motor_direction.map(|d| match d {
                MotorDirection::Clockwise => "CW".into(),
                MotorDirection::CounterClockwise => "CCW".into(),
            }),
            motor_angle: self.motor_angle,
            led_pattern: self.led_pattern.map(|p| format!("{:?}", p)),
            display_on: self.display_on && !self.controller.state.screen.sleep,
            display_title: None, // Now rendered via display buffer
            display_notification: None, // Now rendered via display buffer
            winder_enabled: self.controller.state.winder_enabled,
            status: self.controller.state.status.as_str().into(),
            direction: self.controller.state.direction.as_api_str().into(),
            rotations_per_day: self.controller.state.rotations_per_day,
            timer_enabled: self.controller.state.timer.enabled,
            timer_time: self.controller.state.timer.start_time.to_hh_mm(),
            cycle_progress: self.controller.state.cycle_progress,
            routine_running: self.controller.state.routine.running,
            routine_start_epoch: self.controller.state.routine.start_epoch,
            routine_finish_epoch: self.controller.state.routine.estimated_finish_epoch,
            custom_wind_duration_secs: self.controller.state.custom_wind_duration_secs,
            custom_wind_pause_secs: self.controller.state.custom_wind_pause_secs,
            rotation_duration_secs: self.controller.state.rotation_duration_secs,
            gmt_offset: self.controller.state.rtc.gmt_offset,
            dst: self.controller.state.rtc.dst,
            screen_schedule_enabled: self.controller.state.screen.schedule.enabled,
            screen_schedule_start: self.controller.state.screen.schedule.start.to_hh_mm(),
            screen_schedule_end: self.controller.state.screen.schedule.end.to_hh_mm(),
            screen_sleep: self.controller.state.screen.sleep,
        };
        serde_wasm_bindgen::to_value(&state).unwrap_or(JsValue::NULL)
    }

    /// Get the OLED display frame buffer.
    ///
    /// Returns a Uint8Array of 128*64 = 8192 bytes where each byte is 0 (off) or 1 (on).
    /// This buffer is rendered using the SAME embedded-graphics code as the real firmware.
    #[wasm_bindgen(js_name = getDisplayBuffer)]
    pub fn get_display_buffer(&self) -> Vec<u8> {
        if self.display_on && !self.controller.state.screen.sleep {
            self.sim_display.get_buffer()
        } else {
            // Display is off - return all zeros
            vec![0u8; DISPLAY_WIDTH * DISPLAY_HEIGHT]
        }
    }

    /// Get display dimensions.
    #[wasm_bindgen(js_name = getDisplayWidth)]
    pub fn get_display_width(&self) -> u32 {
        DISPLAY_WIDTH as u32
    }

    #[wasm_bindgen(js_name = getDisplayHeight)]
    pub fn get_display_height(&self) -> u32 {
        DISPLAY_HEIGHT as u32
    }

    /// Get recent trace events for visualization.
    #[wasm_bindgen(js_name = getTrace)]
    pub fn get_trace(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.trace).unwrap_or(JsValue::NULL)
    }

    /// Clear the event trace.
    #[wasm_bindgen(js_name = clearTrace)]
    pub fn clear_trace(&mut self) {
        self.trace.clear();
    }

    /// Advance simulation by one frame (delta_ms milliseconds).
    /// Returns true if state changed.
    #[wasm_bindgen]
    pub fn step(&mut self, delta_ms: u32) -> bool {
        let delta_ms = delta_ms as u64;
        let old_motor = self.motor_running;
        let old_angle = self.motor_angle;

        // Handle pause countdown
        if self.pause_remaining_ms > 0 {
            if delta_ms >= self.pause_remaining_ms {
                let remaining = delta_ms - self.pause_remaining_ms;
                self.now_ms += self.pause_remaining_ms;
                self.pause_remaining_ms = 0;
                self.push_trace(TracedEvent::PauseEnd);
                // Process remaining time
                if remaining > 0 {
                    return self.step(remaining as u32);
                }
            } else {
                self.pause_remaining_ms -= delta_ms;
                self.now_ms += delta_ms;
            }
            return false;
        }

        self.now_ms += delta_ms;

        // Update motor angle for 3D visualization
        if self.motor_running {
            let angle_delta = (delta_ms as f64 / 1000.0) * self.motor_angular_velocity;
            match self.motor_direction {
                Some(MotorDirection::Clockwise) => self.motor_angle += angle_delta,
                Some(MotorDirection::CounterClockwise) => self.motor_angle -= angle_delta,
                None => {}
            }
        }

        // Run firmware tick
        let now_epoch = self.now_ms / 1000;
        let now_time = time_of_day_from_epoch(now_epoch);
        let events = self.controller.tick(now_epoch, now_time);

        self.dispatch_events(events);

        old_motor != self.motor_running || (self.motor_angle - old_angle).abs() > 0.01
    }

    /// Step by one tick interval.
    #[wasm_bindgen(js_name = stepTick)]
    pub fn step_tick(&mut self) -> bool {
        self.step(self.tick_interval_ms as u32)
    }

    /// Apply power toggle.
    #[wasm_bindgen(js_name = setPower)]
    pub fn set_power(&mut self, enabled: bool) {
        let events = self.controller.apply_power(enabled);
        self.dispatch_events(events);
    }

    /// Apply timer enabled toggle.
    #[wasm_bindgen(js_name = setTimerEnabled)]
    pub fn set_timer_enabled(&mut self, enabled: bool) {
        let events = self.controller.apply_timer_enabled(enabled);
        self.dispatch_events(events);
    }

    /// Start winding routine.
    #[wasm_bindgen]
    pub fn start(&mut self) {
        let update = self.build_update(UpdateAction::Start);
        let events = self.controller.apply_update(update, self.now_ms / 1000);
        self.dispatch_events(events);
    }

    /// Stop winding routine.
    #[wasm_bindgen]
    pub fn stop(&mut self) {
        let update = self.build_update(UpdateAction::Stop);
        let events = self.controller.apply_update(update, self.now_ms / 1000);
        self.dispatch_events(events);
    }

    /// Set rotations per day.
    #[wasm_bindgen(js_name = setRotationsPerDay)]
    pub fn set_rotations_per_day(&mut self, rpd: u16) {
        self.controller.state.rotations_per_day = rpd;
    }

    /// Set winding direction ("CW", "CCW", or "BOTH").
    #[wasm_bindgen(js_name = setDirection)]
    pub fn set_direction(&mut self, direction: &str) {
        if let Some(dir) = Direction::from_api(direction) {
            self.controller.state.direction = dir;
        }
    }

    /// Set timer time.
    #[wasm_bindgen(js_name = setTimerTime)]
    pub fn set_timer_time(&mut self, hour: u8, minute: u8) {
        if let Ok(time) = TimeOfDay::new(hour, minute) {
            self.controller.state.timer.start_time = time;
        }
    }

    /// Set custom wind duration.
    #[wasm_bindgen(js_name = setWindDuration)]
    pub fn set_wind_duration(&mut self, secs: u32) {
        self.controller.state.custom_wind_duration_secs = secs;
    }

    /// Set custom wind pause.
    #[wasm_bindgen(js_name = setWindPause)]
    pub fn set_wind_pause(&mut self, secs: u32) {
        self.controller.state.custom_wind_pause_secs = secs;
    }

    /// Request device reset.
    #[wasm_bindgen]
    pub fn reset(&mut self) {
        let events = self.controller.request_reset();
        self.dispatch_events(events);
        // Simulate reboot
        self.restart_from_flash();
    }

    /// Set simulation speed (tick interval in ms).
    #[wasm_bindgen(js_name = setTickInterval)]
    pub fn set_tick_interval(&mut self, ms: u32) {
        let ms = ms as u64;
        self.tick_interval_ms = ms.max(10);
    }

    /// Set motor angular velocity for visualization.
    #[wasm_bindgen(js_name = setMotorSpeed)]
    pub fn set_motor_speed(&mut self, degrees_per_sec: f64) {
        self.motor_angular_velocity = degrees_per_sec;
    }

    /// Reset simulation to initial state.
    #[wasm_bindgen(js_name = resetSim)]
    pub fn reset_sim(&mut self) {
        *self = Self::new();
    }

    /// Set time (jump to specific epoch).
    #[wasm_bindgen(js_name = setTime)]
    pub fn set_time(&mut self, epoch_secs: u32) {
        self.now_ms = (epoch_secs as u64) * 1000;
    }

    fn build_update(&self, action: UpdateAction) -> UpdateRequest {
        let state = &self.controller.state;
        UpdateRequest {
            direction: state.direction,
            rotations_per_day: state.rotations_per_day,
            action,
            hour: state.timer.start_time.hour,
            minutes: state.timer.start_time.minute,
            timer_enabled: state.timer.enabled,
            screen_sleep: state.screen.sleep,
            screen_schedule_enabled: Some(state.screen.schedule.enabled),
            screen_schedule_start: Some(state.screen.schedule.start),
            screen_schedule_end: Some(state.screen.schedule.end),
            custom_wind_duration_secs: state.custom_wind_duration_secs,
            custom_wind_pause_secs: state.custom_wind_pause_secs,
            rotation_duration_secs: state.rotation_duration_secs,
            rtc_gmt_offset: state.rtc.gmt_offset,
            rtc_dst: state.rtc.dst,
        }
    }

    fn dispatch_events(&mut self, events: Vec<ControllerEvent>) {
        for event in events {
            self.handle_event(event);
        }
    }

    fn handle_event(&mut self, event: ControllerEvent) {
        match event {
            ControllerEvent::MotorStart(direction) => {
                self.motor_running = true;
                self.motor_direction = Some(direction);
                self.push_trace(TracedEvent::MotorStart {
                    direction: match direction {
                        MotorDirection::Clockwise => "CW".into(),
                        MotorDirection::CounterClockwise => "CCW".into(),
                    },
                });
            }
            ControllerEvent::MotorStop => {
                self.motor_running = false;
                self.motor_direction = None;
                self.push_trace(TracedEvent::MotorStop);
            }
            ControllerEvent::PauseSeconds(seconds) => {
                self.push_trace(TracedEvent::PauseStart { seconds });
                self.pause_remaining_ms = (seconds as u64) * 1000;
            }
            ControllerEvent::DisplayClear => {
                self.display_on = false;
                self.sim_display.clear();
                self.push_trace(TracedEvent::DisplayClear);
            }
            ControllerEvent::DisplayStatic { title } => {
                self.display_on = true;
                // Update snapshot before drawing
                self.update_display_snapshot();
                self.sim_display.draw_static(&title);
                self.push_trace(TracedEvent::DisplayStatic { title });
            }
            ControllerEvent::DisplayDynamic => {
                // Update snapshot and redraw - uses SAME code as real firmware
                self.update_display_snapshot();
                self.sim_display.draw_dynamic();
                // Intentionally not traced - it's a heartbeat
            }
            ControllerEvent::DisplayNotification(message) => {
                self.update_display_snapshot();
                self.sim_display.notify(&message);
                self.push_trace(TracedEvent::DisplayNotification { message });
            }
            ControllerEvent::Led(pattern) => {
                self.led_pattern = Some(pattern);
                self.push_trace(TracedEvent::Led {
                    pattern: format!("{:?}", pattern),
                });
            }
            ControllerEvent::PersistSettings(snapshot) => {
                self.stored_settings = StoredSettings::from_snapshot(&snapshot);
                self.push_trace(TracedEvent::PersistSettings);
            }
            ControllerEvent::SyncTime => {
                self.push_trace(TracedEvent::SyncTime);
            }
            ControllerEvent::RestartDevice => {
                self.push_trace(TracedEvent::RestartDevice);
            }
        }
    }

    fn push_trace(&mut self, event: TracedEvent) {
        if let Some(last) = self.trace.last_mut() {
            if last.event == event {
                last.count = last.count.saturating_add(1);
                return;
            }
        }

        self.trace.push(TraceEntry { event, count: 1 });
        if self.trace.len() > self.max_trace_entries {
            self.trace.remove(0);
        }
    }

    fn restart_from_flash(&mut self) {
        let state = self
            .stored_settings
            .to_runtime(true)
            .unwrap_or_else(|_| Self::default_runtime_state());
        let rng = XorShift32::new(1);
        self.controller = Controller::new(state, rng);
        self.motor_running = false;
        self.motor_direction = None;
        self.led_pattern = None;
        self.display_on = true;
        self.update_display_snapshot();
        self.sim_display.draw_static("Stopped");
    }

    /// Update the display snapshot from controller state.
    ///
    /// This mirrors how `StatusCache` works in the real firmware.
    fn update_display_snapshot(&mut self) {
        let state = &self.controller.state;
        let mut status = heapless::String::<16>::new();
        let _ = status.push_str(state.status.as_str());

        let mut direction = heapless::String::<8>::new();
        let _ = direction.push_str(state.direction.as_api_str());

        self.sim_display.update_snapshot(DisplaySnapshot {
            status,
            rotations_per_day: state.rotations_per_day,
            direction,
            timer_hour: state.timer.start_time.hour,
            timer_minutes: state.timer.start_time.minute,
            timer_enabled: state.timer.enabled,
        });
    }
}

impl Default for WasmSimulator {
    fn default() -> Self {
        Self::new()
    }
}
