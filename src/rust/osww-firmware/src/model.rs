//! Core domain types for Winderoo runtime state.

use crate::time::TimeOfDay;
use alloc::string::String;
use core::fmt;

/// The user-visible winding direction modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Clockwise only.
    Clockwise,
    /// Counter-clockwise only.
    CounterClockwise,
    /// Alternate between clockwise and counter-clockwise.
    Both,
}

impl Direction {
    /// Parse the API string into a [`Direction`].
    pub fn from_api(value: &str) -> Option<Self> {
        match value {
            "CW" => Some(Self::Clockwise),
            "CCW" => Some(Self::CounterClockwise),
            "BOTH" => Some(Self::Both),
            _ => None,
        }
    }

    /// Render the direction as the API string.
    pub fn as_api_str(&self) -> &'static str {
        match self {
            Self::Clockwise => "CW",
            Self::CounterClockwise => "CCW",
            Self::Both => "BOTH",
        }
    }

    /// Return the Home Assistant selector index.
    pub fn home_assistant_index(&self) -> u8 {
        match self {
            Self::CounterClockwise => 0,
            Self::Both => 1,
            Self::Clockwise => 2,
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_api_str())
    }
}

/// The instantaneous motor direction (not the user-facing mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotorDirection {
    /// Clockwise motor rotation.
    Clockwise,
    /// Counter-clockwise motor rotation.
    CounterClockwise,
}

impl MotorDirection {
    /// Toggle between clockwise and counter-clockwise.
    pub fn toggle(self) -> Self {
        match self {
            Self::Clockwise => Self::CounterClockwise,
            Self::CounterClockwise => Self::Clockwise,
        }
    }
}

/// Winderoo's persisted status text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WinderStatus {
    /// The winding routine is running.
    Winding,
    /// The winding routine is stopped.
    Stopped,
    /// Any custom status that should be preserved as-is.
    Other(String),
}

impl WinderStatus {
    /// Convert a status string into a [`WinderStatus`].
    pub fn from_str(value: &str) -> Self {
        match value {
            "Winding" => Self::Winding,
            "Stopped" => Self::Stopped,
            other => Self::Other(other.to_string()),
        }
    }

    /// Render the status as the UI-facing string.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Winding => "Winding",
            Self::Stopped => "Stopped",
            Self::Other(value) => value.as_str(),
        }
    }
}

/// Timer configuration for scheduled start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerConfig {
    /// Whether the timer is enabled.
    pub enabled: bool,
    /// The time of day to begin winding.
    pub start_time: TimeOfDay,
}

/// RTC configuration settings.
#[derive(Debug, Clone, PartialEq)]
pub struct RtcConfig {
    /// UTC offset in hours (e.g. -5.0).
    pub gmt_offset: f32,
    /// Whether DST should be applied on top of the offset.
    pub dst: bool,
}

/// Screen schedule settings used to auto-sleep the display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenSchedule {
    /// Whether schedule enforcement is active.
    pub enabled: bool,
    /// Start time for the screen to be on.
    pub start: TimeOfDay,
    /// End time for the screen to be on.
    pub end: TimeOfDay,
}

impl ScreenSchedule {
    /// Determine whether the screen should be on at the given time.
    pub fn should_be_awake(&self, now: TimeOfDay) -> bool {
        if !self.enabled {
            return true;
        }
        let current_minutes = now.total_minutes();
        let start_minutes = self.start.total_minutes();
        let end_minutes = self.end.total_minutes();

        if start_minutes <= end_minutes {
            current_minutes >= start_minutes && current_minutes <= end_minutes
        } else {
            current_minutes >= start_minutes || current_minutes <= end_minutes
        }
    }
}

/// Display state and scheduling configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenState {
    /// Whether an OLED screen is physically present.
    pub equipped: bool,
    /// Whether the screen is currently sleeping.
    pub sleep: bool,
    /// The configured schedule for sleep/wake behavior.
    pub schedule: ScreenSchedule,
}

/// Timing details for the active winding routine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineState {
    /// Whether a winding routine is active.
    pub running: bool,
    /// Epoch time when the routine started.
    pub start_epoch: u64,
    /// Epoch time marking the start of the current winding segment.
    ///
    /// This is updated when a "cycle pause" completes so rest windows are scheduled based on
    /// winding time (excluding the pause duration), mirroring the original firmware behavior.
    pub previous_epoch: u64,
    /// Estimated epoch time when the routine should finish.
    pub estimated_finish_epoch: u64,
}

impl RoutineState {
    /// Create an idle routine state.
    pub fn idle() -> Self {
        Self {
            running: false,
            start_epoch: 0,
            previous_epoch: 0,
            estimated_finish_epoch: 0,
        }
    }
}

/// The full mutable runtime state of the firmware.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeState {
    /// Current status text.
    pub status: WinderStatus,
    /// Rotations per day.
    pub rotations_per_day: u16,
    /// The user-selected winding direction mode.
    pub direction: Direction,
    /// The current motor direction when actively winding.
    pub motor_direction: MotorDirection,
    /// Timer settings for automatic starts.
    pub timer: TimerConfig,
    /// Whether the winder is enabled (hard off toggle).
    pub winder_enabled: bool,
    /// Duration between rest periods (seconds).
    pub custom_wind_duration_secs: u32,
    /// Duration of each rest period (seconds).
    pub custom_wind_pause_secs: u32,
    /// Duration of a single rotation (seconds).
    pub rotation_duration_secs: u16,
    /// RTC configuration.
    pub rtc: RtcConfig,
    /// Screen configuration and state.
    pub screen: ScreenState,
    /// Routine timing details.
    pub routine: RoutineState,
    /// Completion ratio of the current cycle, from 0.0 to 1.0.
    pub cycle_progress: f32,
}

impl RuntimeState {
    /// Convenience helper for the UI-facing status string.
    pub fn status_str(&self) -> &str {
        self.status.as_str()
    }
}

/// A minimal snapshot of settings that should be persisted to storage.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsSnapshot {
    /// Current status string.
    pub status: WinderStatus,
    /// Rotations per day.
    pub rotations_per_day: u16,
    /// Timer enabled flag.
    pub timer_enabled: bool,
    /// Timer hour.
    pub timer_hour: u8,
    /// Timer minute.
    pub timer_minutes: u8,
    /// Winding direction.
    pub direction: Direction,
    /// Duration between rest periods (seconds).
    pub custom_wind_duration_secs: u32,
    /// Rest duration (seconds).
    pub custom_wind_pause_secs: u32,
    /// Duration for one rotation (seconds).
    pub rotation_duration_secs: u16,
    /// RTC GMT offset.
    pub gmt_offset: f32,
    /// DST flag.
    pub dst: bool,
    /// Screen schedule enabled flag.
    pub screen_schedule_enabled: bool,
    /// Screen schedule start time.
    pub screen_schedule_start: TimeOfDay,
    /// Screen schedule end time.
    pub screen_schedule_end: TimeOfDay,
    /// Whether the screen is sleeping.
    pub screen_sleep: bool,
}

impl SettingsSnapshot {
    /// Build a persistence snapshot from the current runtime state.
    pub fn from_state(state: &RuntimeState) -> Self {
        Self {
            status: state.status.clone(),
            rotations_per_day: state.rotations_per_day,
            timer_enabled: state.timer.enabled,
            timer_hour: state.timer.start_time.hour,
            timer_minutes: state.timer.start_time.minute,
            direction: state.direction,
            custom_wind_duration_secs: state.custom_wind_duration_secs,
            custom_wind_pause_secs: state.custom_wind_pause_secs,
            rotation_duration_secs: state.rotation_duration_secs,
            gmt_offset: state.rtc.gmt_offset,
            dst: state.rtc.dst,
            screen_schedule_enabled: state.screen.schedule.enabled,
            screen_schedule_start: state.screen.schedule.start,
            screen_schedule_end: state.screen.schedule.end,
            screen_sleep: state.screen.sleep,
        }
    }
}

/// A typed snapshot of the status endpoint values before JSON formatting.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusSnapshot {
    /// Current winding status.
    pub status: WinderStatus,
    /// Rotations per day.
    pub rotations_per_day: u16,
    /// Winding direction.
    pub direction: Direction,
    /// Timer hour.
    pub timer_hour: u8,
    /// Timer minutes.
    pub timer_minutes: u8,
    /// Epoch when winding started.
    pub start_time_epoch: u64,
    /// Current epoch time.
    pub current_time_epoch: u64,
    /// Estimated finish epoch.
    pub estimated_routine_finish_epoch: u64,
    /// Winder enabled flag.
    pub winder_enabled: bool,
    /// Timer enabled flag.
    pub timer_enabled: bool,
    /// WiFi RSSI (dB).
    pub rssi_db: i32,
    /// Whether the screen is asleep.
    pub screen_sleep: bool,
    /// Whether a screen is equipped.
    pub screen_equipped: bool,
    /// Custom wind duration.
    pub custom_wind_duration_secs: u32,
    /// Custom wind pause duration.
    pub custom_wind_pause_secs: u32,
    /// Rotation duration.
    pub rotation_duration_secs: u16,
    /// GMT offset.
    pub gmt_offset: f32,
    /// Firmware version string.
    pub api_version: String,
    /// DST flag.
    pub dst: bool,
    /// Screen schedule enabled flag.
    pub screen_schedule_enabled: bool,
    /// Screen schedule start time.
    pub screen_schedule_start: TimeOfDay,
    /// Screen schedule end time.
    pub screen_schedule_end: TimeOfDay,
}

impl StatusSnapshot {
    /// Build a status snapshot from runtime state and IO-provided values.
    pub fn from_state(
        state: &RuntimeState,
        current_epoch: u64,
        rssi: i32,
        api_version: &str,
    ) -> Self {
        Self {
            status: state.status.clone(),
            rotations_per_day: state.rotations_per_day,
            direction: state.direction,
            timer_hour: state.timer.start_time.hour,
            timer_minutes: state.timer.start_time.minute,
            start_time_epoch: state.routine.start_epoch,
            current_time_epoch: current_epoch,
            estimated_routine_finish_epoch: state.routine.estimated_finish_epoch,
            winder_enabled: state.winder_enabled,
            timer_enabled: state.timer.enabled,
            rssi_db: rssi,
            screen_sleep: state.screen.sleep,
            screen_equipped: state.screen.equipped,
            custom_wind_duration_secs: state.custom_wind_duration_secs,
            custom_wind_pause_secs: state.custom_wind_pause_secs,
            rotation_duration_secs: state.rotation_duration_secs,
            gmt_offset: state.rtc.gmt_offset,
            api_version: api_version.to_string(),
            dst: state.rtc.dst,
            screen_schedule_enabled: state.screen.schedule.enabled,
            screen_schedule_start: state.screen.schedule.start,
            screen_schedule_end: state.screen.schedule.end,
        }
    }
}

/// Parsed update action values from the API layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    /// Start winding.
    Start,
    /// Stop winding.
    Stop,
}

/// Fully parsed update payload used by the controller.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateRequest {
    /// Parsed direction mode.
    pub direction: Direction,
    /// Rotations per day.
    pub rotations_per_day: u16,
    /// Start/stop action.
    pub action: UpdateAction,
    /// Timer hour.
    pub hour: u8,
    /// Timer minutes.
    pub minutes: u8,
    /// Timer enabled flag.
    pub timer_enabled: bool,
    /// Screen sleep state.
    pub screen_sleep: bool,
    /// Screen schedule enabled flag.
    pub screen_schedule_enabled: Option<bool>,
    /// Screen schedule start time.
    pub screen_schedule_start: Option<TimeOfDay>,
    /// Screen schedule end time.
    pub screen_schedule_end: Option<TimeOfDay>,
    /// Custom winding duration (seconds).
    pub custom_wind_duration_secs: u32,
    /// Custom pause duration (seconds).
    pub custom_wind_pause_secs: u32,
    /// Rotation duration (seconds).
    pub rotation_duration_secs: u16,
    /// RTC GMT offset.
    pub rtc_gmt_offset: f32,
    /// RTC DST flag.
    pub rtc_dst: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::TimeOfDay;

    #[test]
    fn direction_round_trip() {
        assert_eq!(Direction::from_api("CW"), Some(Direction::Clockwise));
        assert_eq!(Direction::from_api("CCW"), Some(Direction::CounterClockwise));
        assert_eq!(Direction::from_api("BOTH"), Some(Direction::Both));
        assert_eq!(Direction::Clockwise.as_api_str(), "CW");
    }

    #[test]
    fn screen_schedule_same_day() {
        let schedule = ScreenSchedule {
            enabled: true,
            start: TimeOfDay::new(9, 0).unwrap(),
            end: TimeOfDay::new(17, 0).unwrap(),
        };
        assert!(schedule.should_be_awake(TimeOfDay::new(9, 0).unwrap()));
        assert!(schedule.should_be_awake(TimeOfDay::new(12, 0).unwrap()));
        assert!(!schedule.should_be_awake(TimeOfDay::new(18, 0).unwrap()));
    }

    #[test]
    fn screen_schedule_overnight() {
        let schedule = ScreenSchedule {
            enabled: true,
            start: TimeOfDay::new(22, 0).unwrap(),
            end: TimeOfDay::new(6, 0).unwrap(),
        };
        assert!(schedule.should_be_awake(TimeOfDay::new(23, 30).unwrap()));
        assert!(schedule.should_be_awake(TimeOfDay::new(5, 30).unwrap()));
        assert!(!schedule.should_be_awake(TimeOfDay::new(12, 0).unwrap()));
    }
}
