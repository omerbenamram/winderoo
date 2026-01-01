//! High-level firmware state machine and update logic.

use crate::api::{StatusResponse, UpdateAction, UpdateRequest};
use crate::hardware::{LedPattern, RandomSource};
use crate::model::{Direction, MotorDirection, RuntimeState, WinderStatus};
use crate::settings::StoredSettings;
use crate::time::TimeOfDay;

/// Actions emitted by the controller for the hardware integration layer.
#[derive(Debug, Clone, PartialEq)]
pub enum ControllerEvent {
    /// Start the motor in the given direction.
    MotorStart(MotorDirection),
    /// Stop the motor.
    MotorStop,
    /// Pause winding for the given number of seconds.
    PauseSeconds(u32),
    /// Clear the display buffer.
    DisplayClear,
    /// Draw the static UI chrome with an optional title.
    DisplayStatic { title: String },
    /// Draw the dynamic UI values.
    DisplayDynamic,
    /// Show a notification on the display.
    DisplayNotification(String),
    /// Trigger an LED pattern.
    Led(LedPattern),
    /// Persist settings to storage.
    PersistSettings(StoredSettings),
    /// Synchronize the RTC via NTP.
    SyncTime,
    /// Restart the device.
    RestartDevice,
}

/// Controller that owns the runtime state and emits hardware events.
#[derive(Debug, Clone)]
pub struct Controller<R: RandomSource> {
    /// Mutable runtime state for the firmware.
    pub state: RuntimeState,
    rng: R,
}

impl<R: RandomSource> Controller<R> {
    /// Create a new controller with the given state and RNG.
    pub fn new(state: RuntimeState, rng: R) -> Self {
        Self { state, rng }
    }

    /// Build a status response suitable for `/api/status`.
    pub fn status_response(
        &self,
        current_epoch: u64,
        rssi: i32,
        api_version: &str,
    ) -> StatusResponse {
        StatusResponse::from_state(&self.state, current_epoch, rssi, api_version)
    }

    /// Apply a timer-enabled toggle (from `/api/timer`).
    pub fn apply_timer_enabled(&mut self, enabled: bool) -> Vec<ControllerEvent> {
        self.state.timer.enabled = enabled;
        vec![ControllerEvent::PersistSettings(
            StoredSettings::from_runtime(&self.state),
        )]
    }

    /// Apply a power toggle (from `/api/power`).
    pub fn apply_power(&mut self, enabled: bool) -> Vec<ControllerEvent> {
        self.state.winder_enabled = enabled;
        let mut events = Vec::new();
        if !enabled {
            self.state.status = WinderStatus::Stopped;
            self.state.routine.running = false;
            self.state.cycle_progress = 0.0;
            events.push(ControllerEvent::MotorStop);
            events.push(ControllerEvent::DisplayClear);
        } else if self.state.screen.equipped && !self.state.screen.sleep {
            events.push(ControllerEvent::DisplayStatic {
                title: self.state.status_str().to_string(),
            });
            events.push(ControllerEvent::DisplayDynamic);
        }
        events.push(ControllerEvent::PersistSettings(
            StoredSettings::from_runtime(&self.state),
        ));
        events
    }

    /// Apply a full update payload (from `/api/update`).
    pub fn apply_update(&mut self, update: UpdateRequest, now_epoch: u64) -> Vec<ControllerEvent> {
        let mut events = Vec::new();
        let original_direction = self.state.direction;
        let original_rotations = self.state.rotations_per_day;

        self.state.timer.start_time =
            TimeOfDay::new(update.hour, update.minutes).unwrap_or(self.state.timer.start_time);
        self.state.timer.enabled = update.timer_enabled;
        self.state.custom_wind_duration_secs = update.custom_wind_duration_secs;
        self.state.custom_wind_pause_secs = update.custom_wind_pause_secs;
        self.state.rotation_duration_secs = update.rotation_duration_secs;
        self.state.rtc.gmt_offset = update.rtc_gmt_offset;
        self.state.rtc.dst = update.rtc_dst;
        self.state.screen.sleep = update.screen_sleep;

        if let Some(enabled) = update.screen_schedule_enabled {
            self.state.screen.schedule.enabled = enabled;
        }
        if let Some(start) = update.screen_schedule_start {
            self.state.screen.schedule.start = start;
        }
        if let Some(end) = update.screen_schedule_end {
            self.state.screen.schedule.end = end;
        }

        if update.direction != original_direction {
            self.state.direction = update.direction;
            events.push(ControllerEvent::MotorStop);
            match self.state.direction {
                Direction::Clockwise => self.state.motor_direction = MotorDirection::Clockwise,
                Direction::CounterClockwise => {
                    self.state.motor_direction = MotorDirection::CounterClockwise
                }
                Direction::Both => {}
            }
        }

        if update.rotations_per_day != original_rotations {
            self.state.rotations_per_day = update.rotations_per_day;
            self.state.routine.estimated_finish_epoch =
                now_epoch + calculate_winding_duration_secs(&self.state);
        }

        match update.action {
            UpdateAction::Start => {
                if !self.state.routine.running {
                    events.extend(self.begin_winding(now_epoch));
                }
            }
            UpdateAction::Stop => {
                events.extend(self.stop_winding("Stopped"));
            }
        }

        if self.state.screen.equipped {
            if self.state.screen.sleep {
                events.push(ControllerEvent::DisplayClear);
            } else {
                events.push(ControllerEvent::DisplayStatic {
                    title: self.state.status_str().to_string(),
                });
                events.push(ControllerEvent::DisplayDynamic);
            }
        }

        events.push(ControllerEvent::SyncTime);
        events.push(ControllerEvent::PersistSettings(
            StoredSettings::from_runtime(&self.state),
        ));
        events
    }

    /// Resume winding if the persisted status indicates an active routine.
    pub fn resume_if_needed(&mut self, now_epoch: u64) -> Vec<ControllerEvent> {
        if matches!(self.state.status, WinderStatus::Winding) && self.state.winder_enabled {
            return self.begin_winding(now_epoch);
        }
        Vec::new()
    }

    /// Handle the periodic loop tick.
    pub fn tick(&mut self, now_epoch: u64, now_time: TimeOfDay) -> Vec<ControllerEvent> {
        let mut events = Vec::new();

        if self.state.timer.enabled
            && now_time == self.state.timer.start_time
            && !self.state.routine.running
            && self.state.winder_enabled
        {
            events.extend(self.begin_winding(now_epoch));
            if self.state.screen.equipped && !self.state.screen.sleep {
                events.push(ControllerEvent::DisplayNotification(
                    "Winding Started".to_string(),
                ));
            }
        }

        if self.state.routine.running {
            if now_epoch < self.state.routine.estimated_finish_epoch {
                events.push(ControllerEvent::MotorStart(self.state.motor_direction));
                // Roughly 1/4 of ticks (≈26% with `<= 25`) we *sample* whether it's time to insert a
                // rest/pause window in the winding routine.
                //
                // This is a legacy throttle carried over from the original firmware's tight main loop
                // (where checking every iteration would mean a lot of RTC reads / branching). Functionally,
                // it just adds a small jitter to when the pause boundary is detected.
                let r = self.rng.next_u8() % 100;
                if r <= 25 && self.state.custom_wind_duration_secs > 0 {
                    let elapsed = now_epoch.saturating_sub(self.state.routine.previous_epoch);
                    if elapsed > self.state.custom_wind_duration_secs as u64 {
                        // "Cycle pause": stop the motor and rest for `custom_wind_pause_secs`.
                        // When `direction == Both`, we also toggle motor direction so the next segment
                        // winds the other way.
                        events.push(ControllerEvent::MotorStop);
                        events.push(ControllerEvent::DisplayNotification(
                            "Cycle Pause".to_string(),
                        ));
                        events.push(ControllerEvent::PauseSeconds(
                            self.state.custom_wind_pause_secs,
                        ));

                        // In the original firmware this timestamp is updated after the (blocking) pause
                        // completes; we approximate that here by advancing it by the pause duration.
                        self.state.routine.previous_epoch =
                            now_epoch.saturating_add(self.state.custom_wind_pause_secs as u64);

                        if self.state.direction == Direction::Both {
                            self.state.motor_direction = self.state.motor_direction.toggle();
                        }

                        events.push(ControllerEvent::DisplayNotification("Winding".to_string()));
                        // Restart immediately after the pause (avoids depending on the next tick cadence).
                        events.push(ControllerEvent::MotorStart(self.state.motor_direction));
                    }
                }
            } else {
                self.state.status = WinderStatus::Stopped;
                self.state.routine.running = false;
                self.state.cycle_progress = 0.0;
                events.push(ControllerEvent::MotorStop);
                if self.state.screen.equipped && !self.state.screen.sleep {
                    events.push(ControllerEvent::DisplayNotification(
                        "Winding Complete".to_string(),
                    ));
                }
                events.push(ControllerEvent::PersistSettings(
                    StoredSettings::from_runtime(&self.state),
                ));
            }
        }

        self.update_screen_schedule(now_time, &mut events);
        self.update_cycle_progress(now_epoch);

        if self.state.screen.equipped && !self.state.screen.sleep {
            events.push(ControllerEvent::DisplayDynamic);
        }

        events
    }

    /// Emit events for a reset request.
    pub fn request_reset(&mut self) -> Vec<ControllerEvent> {
        let mut events = Vec::new();
        if self.state.screen.equipped {
            events.push(ControllerEvent::DisplayClear);
            events.push(ControllerEvent::DisplayNotification(
                "Resetting".to_string(),
            ));
        }
        events.push(ControllerEvent::Led(LedPattern::FastBlink));
        events.push(ControllerEvent::RestartDevice);
        events
    }

    fn begin_winding(&mut self, now_epoch: u64) -> Vec<ControllerEvent> {
        self.state.routine.start_epoch = now_epoch;
        self.state.routine.previous_epoch = now_epoch;
        self.state.routine.running = true;
        self.state.status = WinderStatus::Winding;
        self.state.cycle_progress = 0.0;
        self.state.routine.estimated_finish_epoch =
            now_epoch + calculate_winding_duration_secs(&self.state);

        let mut events = Vec::new();
        if self.state.screen.equipped && !self.state.screen.sleep {
            events.push(ControllerEvent::DisplayNotification("Winding".to_string()));
        }
        events
    }

    fn stop_winding(&mut self, message: &str) -> Vec<ControllerEvent> {
        let mut events = Vec::new();
        self.state.status = WinderStatus::Stopped;
        self.state.routine.running = false;
        self.state.cycle_progress = 0.0;
        events.push(ControllerEvent::MotorStop);
        if self.state.screen.equipped && !self.state.screen.sleep {
            events.push(ControllerEvent::DisplayNotification(message.to_string()));
        }
        events
    }

    fn update_screen_schedule(&mut self, now_time: TimeOfDay, events: &mut Vec<ControllerEvent>) {
        if !self.state.screen.equipped {
            return;
        }
        if !self.state.screen.schedule.enabled {
            return;
        }
        let should_be_awake = self.state.screen.schedule.should_be_awake(now_time);
        if should_be_awake && self.state.screen.sleep {
            self.state.screen.sleep = false;
            events.push(ControllerEvent::DisplayClear);
            events.push(ControllerEvent::DisplayStatic {
                title: self.state.status_str().to_string(),
            });
            events.push(ControllerEvent::DisplayDynamic);
        } else if !should_be_awake && !self.state.screen.sleep {
            self.state.screen.sleep = true;
            events.push(ControllerEvent::DisplayClear);
        }
    }

    fn update_cycle_progress(&mut self, now_epoch: u64) {
        if !self.state.routine.running {
            self.state.cycle_progress = 0.0;
            return;
        }
        let total_duration = self
            .state
            .routine
            .estimated_finish_epoch
            .saturating_sub(self.state.routine.start_epoch);
        if total_duration == 0 {
            self.state.cycle_progress = 0.0;
            return;
        }
        let elapsed = now_epoch.saturating_sub(self.state.routine.start_epoch);
        let ratio = (elapsed as f32) / (total_duration as f32);
        self.state.cycle_progress = ratio.clamp(0.0, 1.0);
    }
}

/// Calculate the total winding duration in seconds for the current state.
pub fn calculate_winding_duration_secs(state: &RuntimeState) -> u64 {
    let total_seconds_turning =
        (state.rotations_per_day as u64) * (state.rotation_duration_secs as u64);
    if state.custom_wind_duration_secs == 0 {
        return total_seconds_turning;
    }
    let total_rest_periods = total_seconds_turning / (state.custom_wind_duration_secs as u64);
    let total_rest_duration = total_rest_periods * (state.custom_wind_pause_secs as u64);
    total_seconds_turning + total_rest_duration
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{RandomSource, XorShift32};
    use crate::model::{RoutineState, RtcConfig, ScreenSchedule, ScreenState, TimerConfig};
    use crate::time::TimeOfDay;

    fn base_state() -> RuntimeState {
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

    #[derive(Debug, Clone)]
    struct ConstRng(u8);

    impl RandomSource for ConstRng {
        fn next_u8(&mut self) -> u8 {
            self.0
        }
    }

    #[test]
    fn winding_duration_matches_cxx_formula() {
        let state = base_state();
        let duration = calculate_winding_duration_secs(&state);
        assert_eq!(duration, 1895);
    }

    #[test]
    fn timer_tick_starts_winding() {
        let mut state = base_state();
        state.timer.enabled = true;
        state.timer.start_time = TimeOfDay::new(9, 30).unwrap();
        let mut controller = Controller::new(state, XorShift32::new(1));
        let events = controller.tick(1000, TimeOfDay::new(9, 30).unwrap());
        assert!(controller.state.routine.running);
        assert!(events
            .iter()
            .any(|e| matches!(e, ControllerEvent::DisplayNotification(_))));
    }

    #[test]
    fn apply_update_changes_direction_and_recalculates() {
        let mut controller = Controller::new(base_state(), XorShift32::new(1));
        let update = UpdateRequest {
            direction: Direction::Clockwise,
            rotations_per_day: 300,
            action: UpdateAction::Stop,
            hour: 1,
            minutes: 0,
            timer_enabled: false,
            screen_sleep: false,
            screen_schedule_enabled: None,
            screen_schedule_start: None,
            screen_schedule_end: None,
            custom_wind_duration_secs: 180,
            custom_wind_pause_secs: 15,
            rotation_duration_secs: 8,
            rtc_gmt_offset: 0.0,
            rtc_dst: false,
        };
        let events = controller.apply_update(update, 1000);
        assert_eq!(controller.state.direction, Direction::Clockwise);
        assert!(events
            .iter()
            .any(|e| matches!(e, ControllerEvent::MotorStop)));
        assert_eq!(
            controller.state.routine.estimated_finish_epoch,
            1000 + calculate_winding_duration_secs(&controller.state)
        );
    }

    #[test]
    fn cycle_progress_resets_after_finish() {
        let mut state = base_state();
        state.routine.running = true;
        state.routine.start_epoch = 100;
        state.routine.estimated_finish_epoch = 200;
        let mut controller = Controller::new(state, XorShift32::new(1));
        controller.tick(300, TimeOfDay::new(0, 0).unwrap());
        assert_eq!(controller.state.cycle_progress, 0.0);
    }

    #[test]
    fn cycle_pause_toggles_direction_in_both_mode() {
        let mut state = base_state();
        state.status = WinderStatus::Winding;
        state.routine.running = true;
        state.routine.start_epoch = 0;
        state.routine.previous_epoch = 0;
        state.routine.estimated_finish_epoch = 10_000;
        state.custom_wind_duration_secs = 10;
        state.custom_wind_pause_secs = 5;
        state.direction = Direction::Both;
        state.motor_direction = MotorDirection::Clockwise;

        // Force the random sampling gate to always fire (`r <= 25`).
        let mut controller = Controller::new(state, ConstRng(0));
        let events = controller.tick(20, TimeOfDay::new(0, 0).unwrap());

        assert_eq!(
            controller.state.motor_direction,
            MotorDirection::CounterClockwise
        );
        assert_eq!(controller.state.routine.previous_epoch, 25);

        assert!(events
            .iter()
            .any(|e| matches!(e, ControllerEvent::MotorStop)));
        assert!(events
            .iter()
            .any(|e| matches!(e, ControllerEvent::PauseSeconds(5))));
        assert!(events.iter().any(|e| matches!(
            e,
            ControllerEvent::MotorStart(MotorDirection::CounterClockwise)
        )));
    }

    #[test]
    fn cycle_pause_is_disabled_when_custom_wind_duration_is_zero() {
        let mut state = base_state();
        state.status = WinderStatus::Winding;
        state.routine.running = true;
        state.routine.start_epoch = 0;
        state.routine.previous_epoch = 0;
        state.routine.estimated_finish_epoch = 10_000;
        state.custom_wind_duration_secs = 0;
        state.custom_wind_pause_secs = 5;

        let mut controller = Controller::new(state, ConstRng(0));
        let events = controller.tick(20, TimeOfDay::new(0, 0).unwrap());

        assert!(!events
            .iter()
            .any(|e| matches!(e, ControllerEvent::MotorStop)));
        assert!(!events
            .iter()
            .any(|e| matches!(e, ControllerEvent::PauseSeconds(_))));
    }
}
