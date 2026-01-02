//! API request/response models for the Winderoo firmware.

use alloc::{format, string::{String, ToString}};
use crate::model::{
    Direction, StatusSnapshot, UpdateAction, UpdateRequest,
};
use crate::time::TimeOfDay;
use serde::{Deserialize, Serialize};

/// Errors returned when parsing API payloads into typed requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// A numeric field could not be parsed.
    InvalidNumber { field: &'static str, value: String },
    /// The direction value was not recognized.
    InvalidDirection(String),
    /// The action value was not recognized.
    InvalidAction(String),
    /// The hour/minute values were invalid.
    InvalidTime(String),
    /// A boolean flag was malformed.
    InvalidFlag(String),
}

impl core::fmt::Display for ApiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ApiError::InvalidNumber { field, value } => {
                write!(f, "invalid numeric value for {}: {}", field, value)
            }
            ApiError::InvalidDirection(value) => write!(f, "invalid direction: {}", value),
            ApiError::InvalidAction(value) => write!(f, "invalid action: {}", value),
            ApiError::InvalidTime(value) => write!(f, "invalid time: {}", value),
            ApiError::InvalidFlag(value) => write!(f, "invalid boolean flag: {}", value),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ApiError {}

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
            let int_value = value as i64;
            if (int_value as f64) == value {
                Ok(int_value.to_string())
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
    value
        .parse::<u16>()
        .map_err(|_| ApiError::InvalidNumber { field, value: value.to_string() })
}

fn parse_u32(field: &'static str, value: &str) -> Result<u32, ApiError> {
    value
        .parse::<u32>()
        .map_err(|_| ApiError::InvalidNumber { field, value: value.to_string() })
}

fn parse_u8(field: &'static str, value: &str) -> Result<u8, ApiError> {
    value
        .parse::<u8>()
        .map_err(|_| ApiError::InvalidNumber { field, value: value.to_string() })
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
    #[serde(rename = "customWindPauseDuration", deserialize_with = "de_string_from_any")]
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
            Some(text) => Some(TimeOfDay::parse_hh_mm(&text).map_err(|_| ApiError::InvalidTime(text))?),
            None => None,
        };
        let screen_schedule_end = match value.screen_schedule_end_time {
            Some(text) => Some(TimeOfDay::parse_hh_mm(&text).map_err(|_| ApiError::InvalidTime(text))?),
            None => None,
        };
        let custom_wind_duration_secs = parse_u32("customWindDuration", &value.custom_wind_duration)?;
        let custom_wind_pause_secs = parse_u32("customWindPauseDuration", &value.custom_wind_pause_duration)?;
        let rotation_duration_secs = parse_u16("customDurationInSecondsToCompleteOneRevolution", &value.rotation_duration_secs)?;

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
    /// Build a status response from a typed snapshot.
    pub fn from_snapshot(snapshot: &StatusSnapshot) -> Self {
        Self {
            status: snapshot.status.as_str().to_string(),
            rotations_per_day: snapshot.rotations_per_day.to_string(),
            direction: snapshot.direction.as_api_str().to_string(),
            hour: format!("{:02}", snapshot.timer_hour),
            minutes: format!("{:02}", snapshot.timer_minutes),
            start_time_epoch: snapshot.start_time_epoch,
            current_time_epoch: snapshot.current_time_epoch,
            estimated_routine_finish_epoch: snapshot.estimated_routine_finish_epoch,
            winder_enabled: if snapshot.winder_enabled { "1".to_string() } else { "0".to_string() },
            timer_enabled: if snapshot.timer_enabled { "1".to_string() } else { "0".to_string() },
            db: snapshot.rssi_db,
            screen_sleep: snapshot.screen_sleep,
            screen_equipped: snapshot.screen_equipped,
            custom_wind_duration: snapshot.custom_wind_duration_secs.to_string(),
            custom_wind_pause_duration: snapshot.custom_wind_pause_secs.to_string(),
            rotation_duration_secs: snapshot.rotation_duration_secs,
            gmt_offset: snapshot.gmt_offset,
            api_version: snapshot.api_version.clone(),
            dst: snapshot.dst,
            screen_schedule_enabled: snapshot.screen_schedule_enabled,
            screen_schedule_start_time: snapshot.screen_schedule_start.to_hh_mm(),
            screen_schedule_end_time: snapshot.screen_schedule_end.to_hh_mm(),
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

#[cfg(test)]
mod tests {
    use super::*;

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
