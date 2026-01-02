//! API request/response models for the Winderoo firmware.

use crate::model::{Direction, RuntimeState, WinderStatus};
use crate::settings::StoredSettings;
use crate::time::{epoch_with_offset, TimeOfDay};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors returned when parsing API payloads into typed requests.
#[derive(Debug, Error)]
pub enum ApiError {
    /// A numeric field could not be parsed.
    #[error("invalid numeric value for {field}: {value}")]
    InvalidNumber { field: &'static str, value: String },
    /// The direction value was not recognized.
    #[error("invalid direction: {0}")]
    InvalidDirection(String),
    /// The action value was not recognized.
    #[error("invalid action: {0}")]
    InvalidAction(String),
    /// The hour/minute values were invalid.
    #[error("invalid time: {0}")]
    InvalidTime(String),
    /// A boolean flag was malformed.
    #[error("invalid boolean flag: {0}")]
    InvalidFlag(String),
}

fn de_string_from_any<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Visitor;

    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = String;

        fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
            formatter.write_str("a string or number")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value.to_string())
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value.to_string())
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value.to_string())
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if value.fract() == 0.0 {
                Ok((value as i64).to_string())
            } else {
                Ok(value.to_string())
            }
        }

        fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(if value { "1" } else { "0" }.to_string())
        }
    }

    deserializer.deserialize_any(Visitor)
}

fn de_bool_from_any<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Visitor;

    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = bool;

        fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
            formatter.write_str("a boolean or 0/1")
        }

        fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value != 0)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value != 0)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            match value {
                "1" | "true" | "True" | "TRUE" => Ok(true),
                "0" | "false" | "False" | "FALSE" => Ok(false),
                _ => Err(E::custom("invalid boolean")),
            }
        }
    }

    deserializer.deserialize_any(Visitor)
}

fn parse_u16(field: &'static str, value: &str) -> Result<u16, ApiError> {
    value.parse::<u16>().map_err(|_| ApiError::InvalidNumber {
        field,
        value: value.to_string(),
    })
}

fn parse_u32(field: &'static str, value: &str) -> Result<u32, ApiError> {
    value.parse::<u32>().map_err(|_| ApiError::InvalidNumber {
        field,
        value: value.to_string(),
    })
}

fn parse_u8(field: &'static str, value: &str) -> Result<u8, ApiError> {
    value.parse::<u8>().map_err(|_| ApiError::InvalidNumber {
        field,
        value: value.to_string(),
    })
}

/// Parsed update action values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    /// Start winding.
    Start,
    /// Stop winding.
    Stop,
}

impl UpdateAction {
    /// Parse a string into an [`UpdateAction`].
    pub fn parse(value: &str) -> Result<Self, ApiError> {
        match value {
            "START" => Ok(Self::Start),
            "STOP" => Ok(Self::Stop),
            other => Err(ApiError::InvalidAction(other.to_string())),
        }
    }
}

/// Raw update payload as sent by the frontend API.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdatePayload {
    /// Direction setting.
    #[serde(rename = "rotationDirection")]
    pub rotation_direction: String,
    /// Rotations per day.
    #[serde(rename = "tpd", deserialize_with = "de_string_from_any")]
    pub rotations_per_day: String,
    /// Start/stop action.
    pub action: String,
    /// Timer hour.
    #[serde(deserialize_with = "de_string_from_any")]
    pub hour: String,
    /// Timer minutes.
    #[serde(deserialize_with = "de_string_from_any")]
    pub minutes: String,
    /// Timer enabled flag.
    #[serde(rename = "timerEnabled", deserialize_with = "de_string_from_any")]
    pub timer_enabled: String,
    /// Screen sleep flag.
    #[serde(rename = "screenSleep", deserialize_with = "de_bool_from_any")]
    pub screen_sleep: bool,
    /// Screen schedule enabled flag.
    #[serde(rename = "screenScheduleEnabled")]
    pub screen_schedule_enabled: Option<bool>,
    /// Screen schedule start time.
    #[serde(rename = "screenScheduleStartTime")]
    pub screen_schedule_start_time: Option<String>,
    /// Screen schedule end time.
    #[serde(rename = "screenScheduleEndTime")]
    pub screen_schedule_end_time: Option<String>,
    /// Custom winding duration (seconds).
    #[serde(rename = "customWindDuration", deserialize_with = "de_string_from_any")]
    pub custom_wind_duration: String,
    /// Custom pause duration (seconds).
    #[serde(
        rename = "customWindPauseDuration",
        deserialize_with = "de_string_from_any"
    )]
    pub custom_wind_pause_duration: String,
    /// Rotation duration (seconds).
    #[serde(
        rename = "customDurationInSecondsToCompleteOneRevolution",
        deserialize_with = "de_string_from_any"
    )]
    pub rotation_duration_secs: String,
    /// RTC GMT offset.
    #[serde(rename = "rtcGmtOffset")]
    pub rtc_gmt_offset: f32,
    /// RTC DST flag.
    #[serde(rename = "rtcDST", deserialize_with = "de_bool_from_any")]
    pub rtc_dst: bool,
}

/// Fully parsed update payload.
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

impl TryFrom<UpdatePayload> for UpdateRequest {
    type Error = ApiError;

    fn try_from(value: UpdatePayload) -> Result<Self, Self::Error> {
        let direction = Direction::from_api(&value.rotation_direction)
            .ok_or_else(|| ApiError::InvalidDirection(value.rotation_direction.clone()))?;
        let rotations_per_day = parse_u16("tpd", &value.rotations_per_day)?;
        let action = UpdateAction::parse(&value.action)?;
        let hour = parse_u8("hour", &value.hour)?;
        let minutes = parse_u8("minutes", &value.minutes)?;
        let timer_enabled = match value.timer_enabled.as_str() {
            "1" | "true" | "True" | "TRUE" => true,
            "0" | "false" | "False" | "FALSE" => false,
            other => return Err(ApiError::InvalidFlag(other.to_string())),
        };
        let screen_schedule_start = match value.screen_schedule_start_time {
            Some(text) => {
                Some(TimeOfDay::parse_hh_mm(&text).map_err(|_| ApiError::InvalidTime(text))?)
            }
            None => None,
        };
        let screen_schedule_end = match value.screen_schedule_end_time {
            Some(text) => {
                Some(TimeOfDay::parse_hh_mm(&text).map_err(|_| ApiError::InvalidTime(text))?)
            }
            None => None,
        };
        let custom_wind_duration_secs =
            parse_u32("customWindDuration", &value.custom_wind_duration)?;
        let custom_wind_pause_secs =
            parse_u32("customWindPauseDuration", &value.custom_wind_pause_duration)?;
        let rotation_duration_secs = parse_u16(
            "customDurationInSecondsToCompleteOneRevolution",
            &value.rotation_duration_secs,
        )?;

        Ok(Self {
            direction,
            rotations_per_day,
            action,
            hour,
            minutes,
            timer_enabled,
            screen_sleep: value.screen_sleep,
            screen_schedule_enabled: value.screen_schedule_enabled,
            screen_schedule_start,
            screen_schedule_end,
            custom_wind_duration_secs,
            custom_wind_pause_secs,
            rotation_duration_secs,
            rtc_gmt_offset: value.rtc_gmt_offset,
            rtc_dst: value.rtc_dst,
        })
    }
}

/// Raw power toggle payload.
#[derive(Debug, Clone, Deserialize)]
pub struct PowerPayload {
    /// Whether the winder should be enabled.
    #[serde(rename = "winderEnabled", deserialize_with = "de_bool_from_any")]
    pub winder_enabled: bool,
}

/// Response payload for /api/status.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StatusResponse {
    /// Current winding status.
    pub status: String,
    /// Rotations per day.
    #[serde(rename = "rotationsPerDay")]
    pub rotations_per_day: String,
    /// Winding direction.
    pub direction: String,
    /// Timer hour.
    pub hour: String,
    /// Timer minutes.
    pub minutes: String,
    /// Epoch when winding started.
    #[serde(rename = "startTimeEpoch")]
    pub start_time_epoch: u64,
    /// Current epoch time.
    #[serde(rename = "currentTimeEpoch")]
    pub current_time_epoch: u64,
    /// Estimated finish epoch.
    #[serde(rename = "estimatedRoutineFinishEpoch")]
    pub estimated_routine_finish_epoch: u64,
    /// Winder enabled flag.
    #[serde(rename = "winderEnabled")]
    pub winder_enabled: String,
    /// Timer enabled flag.
    #[serde(rename = "timerEnabled")]
    pub timer_enabled: String,
    /// WiFi RSSI.
    pub db: i32,
    /// Whether the screen is asleep.
    #[serde(rename = "screenSleep")]
    pub screen_sleep: bool,
    /// Whether a screen is equipped.
    #[serde(rename = "screenEquipped")]
    pub screen_equipped: bool,
    /// Custom wind duration.
    #[serde(rename = "customWindDuration")]
    pub custom_wind_duration: String,
    /// Custom wind pause duration.
    #[serde(rename = "customWindPauseDuration")]
    pub custom_wind_pause_duration: String,
    /// Rotation duration.
    #[serde(rename = "customDurationInSecondsToCompleteOneRevolution")]
    pub rotation_duration_secs: u16,
    /// GMT offset.
    #[serde(rename = "gmtOffset")]
    pub gmt_offset: f32,
    /// Firmware version.
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    /// DST flag.
    pub dst: bool,
    /// Screen schedule enabled flag.
    #[serde(rename = "screenScheduleEnabled")]
    pub screen_schedule_enabled: bool,
    /// Screen schedule start time.
    #[serde(rename = "screenScheduleStartTime")]
    pub screen_schedule_start_time: String,
    /// Screen schedule end time.
    #[serde(rename = "screenScheduleEndTime")]
    pub screen_schedule_end_time: String,
}

impl StatusResponse {
    /// Build a status response from runtime state.
    pub fn from_state(
        state: &RuntimeState,
        current_epoch: u64,
        rssi: i32,
        api_version: &str,
    ) -> Self {
        let start_time_epoch = epoch_with_offset(
            state.routine.start_epoch,
            state.rtc.gmt_offset,
            state.rtc.dst,
        );
        let current_time_epoch =
            epoch_with_offset(current_epoch, state.rtc.gmt_offset, state.rtc.dst);
        let estimated_routine_finish_epoch = epoch_with_offset(
            state.routine.estimated_finish_epoch,
            state.rtc.gmt_offset,
            state.rtc.dst,
        );
        Self {
            status: state.status_str().to_string(),
            rotations_per_day: state.rotations_per_day.to_string(),
            direction: state.direction.as_api_str().to_string(),
            hour: format!("{:02}", state.timer.start_time.hour),
            minutes: format!("{:02}", state.timer.start_time.minute),
            start_time_epoch,
            current_time_epoch,
            estimated_routine_finish_epoch,
            winder_enabled: if state.winder_enabled {
                "1".to_string()
            } else {
                "0".to_string()
            },
            timer_enabled: if state.timer.enabled {
                "1".to_string()
            } else {
                "0".to_string()
            },
            db: rssi,
            screen_sleep: state.screen.sleep,
            screen_equipped: state.screen.equipped,
            custom_wind_duration: state.custom_wind_duration_secs.to_string(),
            custom_wind_pause_duration: state.custom_wind_pause_secs.to_string(),
            rotation_duration_secs: state.rotation_duration_secs,
            gmt_offset: state.rtc.gmt_offset,
            api_version: api_version.to_string(),
            dst: state.rtc.dst,
            screen_schedule_enabled: state.screen.schedule.enabled,
            screen_schedule_start_time: state.screen.schedule.start.to_hh_mm(),
            screen_schedule_end_time: state.screen.schedule.end.to_hh_mm(),
        }
    }
}

/// Response payload for /api/reset.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResetResponse {
    /// Reset status message.
    pub status: String,
}

impl ResetResponse {
    /// Build the reset response.
    pub fn new() -> Self {
        Self {
            status: "Resetting".to_string(),
        }
    }
}

/// Helper to transform runtime state into stored settings.
pub fn settings_from_state(state: &RuntimeState) -> StoredSettings {
    StoredSettings::from_runtime(state)
}

/// Helper to create a stored status payload.
pub fn status_string(status: &WinderStatus) -> String {
    status.as_str().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Direction, MotorDirection, RoutineState, RtcConfig, ScreenSchedule, ScreenState,
        TimerConfig, WinderStatus,
    };

    fn base_state() -> RuntimeState {
        RuntimeState {
            status: WinderStatus::Stopped,
            rotations_per_day: 220,
            direction: Direction::Both,
            motor_direction: MotorDirection::CounterClockwise,
            timer: TimerConfig {
                enabled: true,
                start_time: TimeOfDay::new(9, 30).unwrap(),
            },
            winder_enabled: true,
            custom_wind_duration_secs: 180,
            custom_wind_pause_secs: 15,
            rotation_duration_secs: 8,
            rtc: RtcConfig {
                gmt_offset: -5.0,
                dst: true,
            },
            screen: ScreenState {
                equipped: true,
                sleep: false,
                schedule: ScreenSchedule {
                    enabled: true,
                    start: TimeOfDay::new(8, 0).unwrap(),
                    end: TimeOfDay::new(17, 0).unwrap(),
                },
            },
            routine: RoutineState::idle(),
            cycle_progress: 0.25,
        }
    }

    #[test]
    fn update_payload_parses_mixed_types() {
        let json = r#"
        {
            "rotationDirection": "CW",
            "tpd": 240,
            "action": "START",
            "hour": 7,
            "minutes": "05",
            "timerEnabled": 1,
            "screenSleep": false,
            "screenScheduleEnabled": true,
            "screenScheduleStartTime": "09:00",
            "screenScheduleEndTime": "17:00",
            "customWindDuration": 180,
            "customWindPauseDuration": "15",
            "customDurationInSecondsToCompleteOneRevolution": 8,
            "rtcGmtOffset": -4.0,
            "rtcDST": true
        }
        "#;

        let payload: UpdatePayload = serde_json::from_str(json).expect("payload");
        let request = UpdateRequest::try_from(payload).expect("request");

        assert_eq!(request.direction, Direction::Clockwise);
        assert_eq!(request.rotations_per_day, 240);
        assert_eq!(request.hour, 7);
        assert_eq!(request.minutes, 5);
        assert!(request.timer_enabled);
        assert_eq!(request.custom_wind_duration_secs, 180);
        assert_eq!(request.custom_wind_pause_secs, 15);
        assert_eq!(request.rotation_duration_secs, 8);
        assert_eq!(request.rtc_gmt_offset, -4.0);
        assert!(request.rtc_dst);
        assert_eq!(
            request.screen_schedule_start.unwrap(),
            TimeOfDay::new(9, 0).unwrap()
        );
    }

    #[test]
    fn power_payload_accepts_numeric() {
        let json = r#"{ "winderEnabled": 0 }"#;
        let payload: PowerPayload = serde_json::from_str(json).expect("payload");
        assert!(!payload.winder_enabled);
    }

    #[test]
    fn status_response_formats_state() {
        let state = base_state();
        let response = StatusResponse::from_state(&state, 1000, -42, "4.0.1");
        assert_eq!(response.status, "Stopped");
        assert_eq!(response.rotations_per_day, "220");
        assert_eq!(response.direction, "BOTH");
        assert_eq!(response.hour, "09");
        assert_eq!(response.minutes, "30");
        assert_eq!(response.timer_enabled, "1");
        assert_eq!(response.screen_schedule_start_time, "08:00");
    }

    #[test]
    fn update_payload_parses_strings() {
        let json = r#"{
            "rotationDirection": "BOTH",
            "tpd": "220",
            "action": "START",
            "hour": "09",
            "minutes": "30",
            "timerEnabled": "1",
            "screenSleep": false,
            "customWindDuration": "180",
            "customWindPauseDuration": "15",
            "customDurationInSecondsToCompleteOneRevolution": 8,
            "rtcGmtOffset": 0,
            "rtcDST": false
        }"#;
        let payload: UpdatePayload = serde_json::from_str(json).expect("payload");
        let request = UpdateRequest::try_from(payload).expect("request");
        assert_eq!(request.rotations_per_day, 220);
        assert_eq!(request.action, UpdateAction::Start);
        assert!(request.timer_enabled);
    }

    #[test]
    fn power_payload_parses_numeric() {
        let json = r#"{ "winderEnabled": 1 }"#;
        let payload: PowerPayload = serde_json::from_str(json).expect("payload");
        assert!(payload.winder_enabled);
    }
}
