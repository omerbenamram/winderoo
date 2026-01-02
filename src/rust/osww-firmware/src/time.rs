//! Time-related helpers for the firmware domain model.

use alloc::{format, string::{String, ToString}};
use core::fmt;

/// Errors returned when parsing or constructing a [`TimeOfDay`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeError {
    /// Hour was outside the 0-23 range.
    InvalidHour(u8),
    /// Minute was outside the 0-59 range.
    InvalidMinute(u8),
    /// The input string was not in the expected `HH:MM` format.
    InvalidFormat(String),
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeError::InvalidHour(hour) => write!(f, "hour out of range: {}", hour),
            TimeError::InvalidMinute(minute) => write!(f, "minute out of range: {}", minute),
            TimeError::InvalidFormat(value) => write!(f, "invalid time format: {}", value),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for TimeError {}

/// A 24-hour clock time without a date or timezone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeOfDay {
    /// Hour in 24-hour form (0-23).
    pub hour: u8,
    /// Minute (0-59).
    pub minute: u8,
}

impl TimeOfDay {
    /// Construct a new [`TimeOfDay`] after validating the hour and minute ranges.
    pub fn new(hour: u8, minute: u8) -> Result<Self, TimeError> {
        if hour > 23 {
            return Err(TimeError::InvalidHour(hour));
        }
        if minute > 59 {
            return Err(TimeError::InvalidMinute(minute));
        }
        Ok(Self { hour, minute })
    }

    /// Parse a `HH:MM` string into a [`TimeOfDay`].
    pub fn parse_hh_mm(input: &str) -> Result<Self, TimeError> {
        let mut parts = input.split(':');
        let hour_str = parts.next().ok_or_else(|| TimeError::InvalidFormat(input.to_string()))?;
        let minute_str = parts.next().ok_or_else(|| TimeError::InvalidFormat(input.to_string()))?;
        if parts.next().is_some() {
            return Err(TimeError::InvalidFormat(input.to_string()));
        }
        let hour: u8 = hour_str
            .parse()
            .map_err(|_| TimeError::InvalidFormat(input.to_string()))?;
        let minute: u8 = minute_str
            .parse()
            .map_err(|_| TimeError::InvalidFormat(input.to_string()))?;
        Self::new(hour, minute)
    }

    /// Render the time as a zero-padded `HH:MM` string.
    pub fn to_hh_mm(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    /// Total minutes since midnight.
    pub fn total_minutes(&self) -> u16 {
        (self.hour as u16) * 60 + (self.minute as u16)
    }
}

/// Convert an epoch timestamp (seconds) into a [`TimeOfDay`], wrapping every 24 hours.
pub fn time_of_day_from_epoch(epoch: u64) -> TimeOfDay {
    let seconds = (epoch % 86_400) as u32;
    let hour = (seconds / 3_600) as u8;
    let minute = ((seconds / 60) % 60) as u8;
    TimeOfDay::new(hour, minute).unwrap_or(TimeOfDay { hour: 0, minute: 0 })
}

/// Convert a UTC epoch timestamp into a local epoch timestamp using a GMT offset and DST flag.
///
/// Negative results clamp to `0`.
pub fn local_epoch_from_utc(utc_epoch: u64, gmt_offset_hours: f32, dst: bool) -> u64 {
    let mut offset_hours = gmt_offset_hours;
    if dst {
        offset_hours += 1.0;
    }
    // `f32::round` is not available in all `no_std` targets, so implement a tiny
    // "round half away from zero" helper using truncating casts.
    let offset_secs_f = offset_hours * 3600.0;
    let offset_secs = if offset_secs_f >= 0.0 {
        (offset_secs_f + 0.5) as i64
    } else {
        (offset_secs_f - 0.5) as i64
    };
    let local_epoch = utc_epoch as i64 + offset_secs;
    if local_epoch < 0 {
        0
    } else {
        local_epoch as u64
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hh_mm())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_time() {
        let time = TimeOfDay::parse_hh_mm("09:05").expect("valid time");
        assert_eq!(time.hour, 9);
        assert_eq!(time.minute, 5);
        assert_eq!(time.to_hh_mm(), "09:05");
    }

    #[test]
    fn reject_invalid_time() {
        assert!(TimeOfDay::parse_hh_mm("24:00").is_err());
        assert!(TimeOfDay::parse_hh_mm("10:60").is_err());
        assert!(TimeOfDay::parse_hh_mm("10").is_err());
        assert!(TimeOfDay::parse_hh_mm("10:00:00").is_err());
    }

    #[test]
    fn total_minutes() {
        let time = TimeOfDay::new(2, 30).unwrap();
        assert_eq!(time.total_minutes(), 150);
    }

    #[test]
    fn time_of_day_from_epoch_wraps_daily() {
        let time = time_of_day_from_epoch(86_400 + 3_660);
        assert_eq!(time.hour, 1);
        assert_eq!(time.minute, 1);
    }

    #[test]
    fn local_epoch_from_utc_applies_offset() {
        assert_eq!(local_epoch_from_utc(100, 1.0, false), 3_700);
        assert_eq!(local_epoch_from_utc(100, 1.0, true), 7_300);
        assert_eq!(local_epoch_from_utc(100, -1.0, false), 0);
    }
}
