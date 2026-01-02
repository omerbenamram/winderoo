use std::string::{String, ToString};
use std::vec::Vec;

use winderoo_firmware::controller::{Controller, ControllerEvent};
use winderoo_firmware::hardware::{LedPattern, XorShift32};
use winderoo_firmware::model::{
    MotorDirection, SettingsSnapshot, StatusSnapshot, UpdateRequest, WinderStatus,
};
use winderoo_firmware::settings::StoredSettings;
use winderoo_firmware::time::{time_of_day_from_epoch, TimeOfDay};

/// Configuration for the simulator runtime.
#[derive(Debug, Clone)]
pub struct SimConfig {
    /// Whether the simulated device has a screen installed.
    pub screen_equipped: bool,
    /// Seed used for the controller RNG.
    pub rng_seed: u32,
    /// Tick cadence (ms) used by [`SimEngine::step_tick`].
    pub tick_interval_ms: u64,
    /// RSSI value (dB) shown in status snapshots.
    pub rssi_db: i32,
    /// API version string shown in status snapshots.
    pub api_version: String,
    /// Maximum number of log entries to keep.
    pub max_log_entries: usize,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            screen_equipped: true,
            rng_seed: 1,
            tick_interval_ms: 500,
            rssi_db: -42,
            api_version: "sim".to_string(),
            max_log_entries: 2_000,
        }
    }
}

/// A single event/log entry produced during simulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimLogEntry {
    /// Timestamp at which the entry was recorded (ms since epoch 0 of the sim).
    pub t_ms: u64,
    /// Human-readable message.
    pub message: String,
}

impl SimLogEntry {
    fn new(t_ms: u64, message: impl Into<String>) -> Self {
        Self {
            t_ms,
            message: message.into(),
        }
    }
}

/// Simulated motor device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimMotor {
    /// Whether the motor is currently running.
    pub running: bool,
    /// Current motor direction if running.
    pub direction: Option<MotorDirection>,
    /// Number of times the motor was started.
    pub starts: u64,
    /// Number of times the motor was stopped.
    pub stops: u64,
}

/// Simulated LED device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimLed {
    /// Last applied LED pattern.
    pub last_pattern: Option<LedPattern>,
    /// Count of pattern applications.
    pub pattern_count: u64,
}

/// Simulated display device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimDisplay {
    /// Current static title (if any).
    pub title: Option<String>,
    /// Last notification message (if any).
    pub last_notification: Option<String>,
    /// Number of times the display was cleared.
    pub clears: u64,
    /// Number of dynamic renders requested.
    pub dynamic_renders: u64,
}

/// Simulated system hooks (persistence/time sync/restart).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimSystem {
    /// Last settings snapshot persisted by the controller.
    pub last_persisted: Option<SettingsSnapshot>,
    /// Whether time sync was requested.
    pub sync_time_requested: u64,
    /// How many restarts were requested.
    pub restart_requests: u64,
}

/// Deterministic firmware simulation engine.
///
/// This runs the real `winderoo-firmware` controller and interprets the emitted
/// `ControllerEvent`s into simulated IO state.
#[derive(Debug, Clone)]
pub struct SimEngine {
    config: SimConfig,
    /// Simulated clock (ms since simulator start).
    now_ms: u64,
    /// Flash-backed settings (simulated).
    stored_settings: StoredSettings,

    controller: Controller<XorShift32>,
    motor: SimMotor,
    led: SimLed,
    display: SimDisplay,
    system: SimSystem,
    log: Vec<SimLogEntry>,
}

impl Default for SimEngine {
    fn default() -> Self {
        Self::new(SimConfig::default())
    }
}

impl SimEngine {
    /// Create a new simulation engine with default stored settings.
    pub fn new(config: SimConfig) -> Self {
        let stored_settings = StoredSettings::default();
        Self::from_stored_settings(config, stored_settings)
    }

    /// Create a new simulation engine from an explicit settings blob (simulated flash).
    pub fn from_stored_settings(config: SimConfig, stored_settings: StoredSettings) -> Self {
        let state = stored_settings
            .to_runtime(config.screen_equipped)
            .unwrap_or_else(|_| StoredSettings::default().to_runtime(config.screen_equipped).expect("default runtime"));
        let rng = XorShift32::new(config.rng_seed);
        let controller = Controller::new(state, rng);

        Self {
            config,
            now_ms: 0,
            stored_settings,
            controller,
            motor: SimMotor::default(),
            led: SimLed::default(),
            display: SimDisplay::default(),
            system: SimSystem::default(),
            log: Vec::new(),
        }
    }

    /// Read-only access to the simulation config.
    pub fn config(&self) -> &SimConfig {
        &self.config
    }

    /// Current simulation time in milliseconds.
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// Current simulation epoch time in seconds.
    pub fn now_epoch(&self) -> u64 {
        self.now_ms / 1000
    }

    /// Current simulation time-of-day (derived from epoch seconds).
    pub fn now_time_of_day(&self) -> TimeOfDay {
        time_of_day_from_epoch(self.now_epoch())
    }

    /// Access the controller state (clone) for display.
    pub fn runtime_state(&self) -> winderoo_firmware::model::RuntimeState {
        self.controller.state.clone()
    }

    /// Access simulated motor state.
    pub fn motor(&self) -> &SimMotor {
        &self.motor
    }

    /// Access simulated LED state.
    pub fn led(&self) -> &SimLed {
        &self.led
    }

    /// Access simulated display state.
    pub fn display(&self) -> &SimDisplay {
        &self.display
    }

    /// Access simulated system state.
    pub fn system(&self) -> &SimSystem {
        &self.system
    }

    /// Access the event log.
    pub fn log(&self) -> &[SimLogEntry] {
        &self.log
    }

    /// Clear the log.
    pub fn clear_log(&mut self) {
        self.log.clear();
    }

    /// Build the status snapshot the device would expose via `/api/status`.
    pub fn status_snapshot(&self) -> StatusSnapshot {
        self.controller
            .status_snapshot(self.now_epoch(), self.config.rssi_db, &self.config.api_version)
    }

    /// Step the simulation forward by one firmware tick.
    pub fn step_tick(&mut self) {
        self.now_ms = self.now_ms.saturating_add(self.config.tick_interval_ms);
        self.tick_once();
    }

    /// Advance time without ticking (useful for testing).
    #[allow(dead_code)]
    pub fn advance_ms(&mut self, delta_ms: u64) {
        self.now_ms = self.now_ms.saturating_add(delta_ms);
    }

    /// Run one controller tick at the current simulated time.
    pub fn tick_once(&mut self) {
        let now_epoch = self.now_epoch();
        let now_time = self.now_time_of_day();
        let events = self.controller.tick(now_epoch, now_time);
        self.dispatch_controller_events(events);
    }

    /// Apply a power toggle (maps to `/api/power`).
    pub fn apply_power(&mut self, enabled: bool) {
        let events = self.controller.apply_power(enabled);
        self.dispatch_controller_events(events);
    }

    /// Apply timer enabled flag (maps to `/api/timer`).
    pub fn apply_timer_enabled(&mut self, enabled: bool) {
        let events = self.controller.apply_timer_enabled(enabled);
        self.dispatch_controller_events(events);
    }

    /// Apply a full update payload (maps to `/api/update`).
    pub fn apply_update(&mut self, update: UpdateRequest) {
        let events = self.controller.apply_update(update, self.now_epoch());
        self.dispatch_controller_events(events);
    }

    /// Request a reset (maps to `/api/reset`).
    pub fn request_reset(&mut self) {
        let events = self.controller.request_reset();
        self.dispatch_controller_events(events);
    }

    /// Dispatch controller events (including pause semantics).
    pub fn dispatch_controller_events<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = ControllerEvent>,
    {
        for event in events {
            self.handle_event(event);
        }
    }

    fn push_log(&mut self, message: impl Into<String>) {
        self.log.push(SimLogEntry::new(self.now_ms, message));
        let max = self.config.max_log_entries;
        if max > 0 && self.log.len() > max {
            let excess = self.log.len() - max;
            self.log.drain(0..excess);
        }
    }

    fn handle_event(&mut self, event: ControllerEvent) {
        match event {
            ControllerEvent::MotorStart(direction) => {
                self.motor.running = true;
                self.motor.direction = Some(direction);
                self.motor.starts = self.motor.starts.saturating_add(1);
                self.push_log(format!("MotorStart({direction:?})"));
            }
            ControllerEvent::MotorStop => {
                self.motor.running = false;
                self.motor.direction = None;
                self.motor.stops = self.motor.stops.saturating_add(1);
                self.push_log("MotorStop");
            }
            ControllerEvent::PauseSeconds(seconds) => {
                self.push_log(format!("PauseSeconds({seconds})"));
                // Important: this advances the simulated clock but does NOT run controller ticks during the pause,
                // matching the embedded `ControllerTask` behavior (the task awaits the timer).
                self.now_ms = self.now_ms.saturating_add((seconds as u64).saturating_mul(1000));
            }
            ControllerEvent::DisplayClear => {
                self.display.clears = self.display.clears.saturating_add(1);
                self.display.title = None;
                self.display.last_notification = None;
                self.push_log("DisplayClear");
            }
            ControllerEvent::DisplayStatic { title } => {
                self.display.title = Some(title.clone());
                self.push_log(format!("DisplayStatic({title})"));
            }
            ControllerEvent::DisplayDynamic => {
                self.display.dynamic_renders = self.display.dynamic_renders.saturating_add(1);
            }
            ControllerEvent::DisplayNotification(message) => {
                self.display.last_notification = Some(message.clone());
                self.push_log(format!("DisplayNotification({message})"));
            }
            ControllerEvent::Led(pattern) => {
                self.led.last_pattern = Some(pattern);
                self.led.pattern_count = self.led.pattern_count.saturating_add(1);
                self.push_log(format!("Led({pattern:?})"));
            }
            ControllerEvent::PersistSettings(snapshot) => {
                self.system.last_persisted = Some(snapshot.clone());
                self.stored_settings = StoredSettings::from_snapshot(&snapshot);
                self.push_log("PersistSettings");
            }
            ControllerEvent::SyncTime => {
                self.system.sync_time_requested = self.system.sync_time_requested.saturating_add(1);
                self.push_log("SyncTime");
            }
            ControllerEvent::RestartDevice => {
                self.system.restart_requests = self.system.restart_requests.saturating_add(1);
                self.push_log("RestartDevice");
                self.restart_from_flash();
            }
        }
    }

    fn restart_from_flash(&mut self) {
        // Mirror "reboot": reset simulated IO and re-load runtime state from persisted settings.
        let state = self
            .stored_settings
            .to_runtime(self.config.screen_equipped)
            .unwrap_or_else(|_| StoredSettings::default().to_runtime(self.config.screen_equipped).expect("default runtime"));
        let rng = XorShift32::new(self.config.rng_seed);
        self.controller = Controller::new(state, rng);

        self.motor = SimMotor::default();
        self.led = SimLed::default();
        self.display = SimDisplay::default();

        // If the previous run asked for "Resetting" etc, the reboot happens after that.
        // We keep the log and timestamp.
        self.push_log("Rebooted");
    }

    /// Convenience helper: build an `UpdateRequest` matching the current state, overriding the action.
    pub fn update_from_state_with_action(
        &self,
        action: winderoo_firmware::model::UpdateAction,
    ) -> UpdateRequest {
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

    /// High-level convenience: whether the controller reports a running routine.
    #[allow(dead_code)]
    pub fn is_winding(&self) -> bool {
        matches!(self.controller.state.status, WinderStatus::Winding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_seconds_advances_clock_and_delays_subsequent_events() {
        let mut sim = SimEngine::default();
        assert_eq!(sim.now_ms(), 0);

        sim.dispatch_controller_events([
            ControllerEvent::PauseSeconds(3),
            ControllerEvent::MotorStart(MotorDirection::Clockwise),
        ]);

        assert_eq!(sim.now_ms(), 3_000);
        assert!(sim.motor().running);
        assert_eq!(sim.motor().direction, Some(MotorDirection::Clockwise));
    }

    #[test]
    fn restart_reloads_persisted_settings() {
        let mut sim = SimEngine::default();

        // Persist a change into "flash".
        sim.apply_power(false);
        assert!(!sim.runtime_state().winder_enabled);
        assert!(sim.system().last_persisted.is_some());

        // Simulate reboot.
        sim.dispatch_controller_events([ControllerEvent::RestartDevice]);

        // State should be loaded from persisted settings.
        assert!(!sim.runtime_state().winder_enabled);
    }
}

