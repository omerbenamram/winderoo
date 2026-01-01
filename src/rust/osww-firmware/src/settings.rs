//! Persistent settings handling for Winderoo.

use crate::model::{
    Direction, MotorDirection, RtcConfig, RuntimeState, ScreenSchedule, ScreenState, SettingsSnapshot,
    TimerConfig, WinderStatus,
};
use crate::time::{TimeError, TimeOfDay};
use alloc::string::String;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Settings as stored on the device filesystem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredSettings {
    /// Saved status string ("Winding" or "Stopped").
    #[serde(rename = "savedStatus")]
    pub status: String,
    /// Saved rotations per day.
    #[serde(rename = "savedTPD")]
    pub rotations_per_day: String,
    /// Saved timer hour.
    #[serde(rename = "savedHour")]
    pub hour: String,
    /// Saved timer minute.
    #[serde(rename = "savedMinutes")]
    pub minutes: String,
    /// Saved timer enabled state ("0" or "1").
    #[serde(rename = "savedTimerState")]
    pub timer_enabled: String,
    /// Saved winding direction.
    #[serde(rename = "savedDirection")]
    pub direction: String,
    /// Saved duration between rest periods (seconds).
    #[serde(rename = "customWindDuration")]
    pub custom_wind_duration: String,
    /// Saved rest duration (seconds).
    #[serde(rename = "customWindPauseDuration")]
    pub custom_wind_pause_duration: String,
    /// Saved duration for a single rotation (seconds).
    #[serde(rename = "customDurationInSecondsToCompleteOneRevolution")]
    pub rotation_duration_secs: u16,
    /// Saved GMT offset.
    #[serde(rename = "gmtOffset")]
    pub gmt_offset: f32,
    /// Saved DST toggle.
    #[serde(rename = "dst")]
    pub dst: bool,
    /// Saved screen schedule enabled flag.
    #[serde(rename = "screenScheduleEnabled")]
    pub screen_schedule_enabled: bool,
    /// Saved schedule start time.
    #[serde(rename = "screenScheduleStartTime")]
    pub screen_schedule_start_time: String,
    /// Saved schedule end time.
    #[serde(rename = "screenScheduleEndTime")]
    pub screen_schedule_end_time: String,
    /// Saved screen sleep flag.
    #[serde(rename = "screenSleep")]
    pub screen_sleep: bool,
}

impl Default for StoredSettings {
    fn default() -> Self {
        Self {
            status: "Stopped".to_string(),
            rotations_per_day: "220".to_string(),
            hour: "00".to_string(),
            minutes: "00".to_string(),
            timer_enabled: "0".to_string(),
            direction: "BOTH".to_string(),
            custom_wind_duration: "180".to_string(),
            custom_wind_pause_duration: "15".to_string(),
            rotation_duration_secs: 8,
            gmt_offset: 0.0,
            dst: false,
            screen_schedule_enabled: false,
            screen_schedule_start_time: "00:00".to_string(),
            screen_schedule_end_time: "00:00".to_string(),
            screen_sleep: false,
        }
    }
}

/// Errors that can occur when parsing stored settings into runtime state.
#[derive(Debug, Error)]
pub enum SettingsError {
    /// A numeric field could not be parsed.
    #[error("invalid numeric value for {field}: {value}")]
    InvalidNumber { field: &'static str, value: String },
    /// The direction value was not recognized.
    #[error("invalid direction: {0}")]
    InvalidDirection(String),
    /// A boolean flag was malformed.
    #[error("invalid boolean flag: {0}")]
    InvalidFlag(String),
    /// A time value was invalid.
    #[error("invalid time: {0}")]
    InvalidTime(String),
    /// A time parsing error was raised.
    #[error(transparent)]
    Time(#[from] TimeError),
}

fn parse_u16(field: &'static str, value: &str) -> Result<u16, SettingsError> {
    value
        .parse::<u16>()
        .map_err(|_| SettingsError::InvalidNumber { field, value: value.to_string() })
}

fn parse_u32(field: &'static str, value: &str) -> Result<u32, SettingsError> {
    value
        .parse::<u32>()
        .map_err(|_| SettingsError::InvalidNumber { field, value: value.to_string() })
}

fn parse_u8(field: &'static str, value: &str) -> Result<u8, SettingsError> {
    value
        .parse::<u8>()
        .map_err(|_| SettingsError::InvalidNumber { field, value: value.to_string() })
}

fn parse_bool_flag(value: &str) -> Result<bool, SettingsError> {
    match value {
        "1" | "true" | "True" | "TRUE" => Ok(true),
        "0" | "false" | "False" | "FALSE" => Ok(false),
        other => Err(SettingsError::InvalidFlag(other.to_string())),
    }
}

impl StoredSettings {
    /// Convert stored settings into a fully typed runtime state.
    pub fn to_runtime(&self, screen_equipped: bool) -> Result<RuntimeState, SettingsError> {
        let status = WinderStatus::from_str(&self.status);
        let rotations_per_day = parse_u16("rotations_per_day", &self.rotations_per_day)?;
        let direction = Direction::from_api(&self.direction)
            .ok_or_else(|| SettingsError::InvalidDirection(self.direction.clone()))?;
        let motor_direction = match direction {
            Direction::Clockwise => MotorDirection::Clockwise,
            Direction::CounterClockwise => MotorDirection::CounterClockwise,
            Direction::Both => MotorDirection::CounterClockwise,
        };
        let timer_enabled = parse_bool_flag(&self.timer_enabled)?;
        let hour = parse_u8("hour", &self.hour)?;
        let minute = parse_u8("minutes", &self.minutes)?;
        let timer = TimerConfig {
            enabled: timer_enabled,
            start_time: TimeOfDay::new(hour, minute)?,
        };
        let custom_wind_duration_secs = parse_u32("custom_wind_duration", &self.custom_wind_duration)?;
        let custom_wind_pause_secs = parse_u32("custom_wind_pause_duration", &self.custom_wind_pause_duration)?;
        let rotation_duration_secs = self.rotation_duration_secs;
        let rtc = RtcConfig {
            gmt_offset: self.gmt_offset,
            dst: self.dst,
        };
        let schedule = ScreenSchedule {
            enabled: self.screen_schedule_enabled,
            start: TimeOfDay::parse_hh_mm(&self.screen_schedule_start_time)?,
            end: TimeOfDay::parse_hh_mm(&self.screen_schedule_end_time)?,
        };
        let screen = ScreenState {
            equipped: screen_equipped,
            sleep: self.screen_sleep,
            schedule,
        };

        Ok(RuntimeState {
            status,
            rotations_per_day,
            direction,
            motor_direction,
            timer,
            winder_enabled: true,
            custom_wind_duration_secs,
            custom_wind_pause_secs,
            rotation_duration_secs,
            rtc,
            screen,
            routine: crate::model::RoutineState::idle(),
            cycle_progress: 0.0,
        })
    }

    /// Create stored settings from a typed snapshot.
    pub fn from_snapshot(snapshot: &SettingsSnapshot) -> Self {
        Self {
            status: snapshot.status.as_str().to_string(),
            rotations_per_day: snapshot.rotations_per_day.to_string(),
            hour: format!("{:02}", snapshot.timer_hour),
            minutes: format!("{:02}", snapshot.timer_minutes),
            timer_enabled: if snapshot.timer_enabled { "1".to_string() } else { "0".to_string() },
            direction: snapshot.direction.as_api_str().to_string(),
            custom_wind_duration: snapshot.custom_wind_duration_secs.to_string(),
            custom_wind_pause_duration: snapshot.custom_wind_pause_secs.to_string(),
            rotation_duration_secs: snapshot.rotation_duration_secs,
            gmt_offset: snapshot.gmt_offset,
            dst: snapshot.dst,
            screen_schedule_enabled: snapshot.screen_schedule_enabled,
            screen_schedule_start_time: snapshot.screen_schedule_start.to_hh_mm(),
            screen_schedule_end_time: snapshot.screen_schedule_end.to_hh_mm(),
            screen_sleep: snapshot.screen_sleep,
        }
    }

    /// Create stored settings from the current runtime state.
    pub fn from_runtime(state: &RuntimeState) -> Self {
        Self::from_snapshot(&SettingsSnapshot::from_state(state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_round_trip() {
        let stored = StoredSettings::default();
        let runtime = stored.to_runtime(true).expect("runtime state");
        let stored_again = StoredSettings::from_runtime(&runtime);
        assert_eq!(stored.status, stored_again.status);
        assert_eq!(stored.rotations_per_day, stored_again.rotations_per_day);
        assert_eq!(stored.direction, stored_again.direction);
    }

    #[test]
    fn parse_timer_enabled_flag() {
        let mut stored = StoredSettings::default();
        stored.timer_enabled = "1".to_string();
        let runtime = stored.to_runtime(false).expect("runtime state");
        assert!(runtime.timer.enabled);
    }
}
